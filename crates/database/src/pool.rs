use crate::runtime;
use anyhow::{Context, Result};
use std::{
    collections::HashMap,
    future::Future,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
    time::Instant,
};
use tokio::sync::{OwnedSemaphorePermit, RwLock, RwLockWriteGuard, Semaphore};
use turso::Connection;

const ACTIVE_OPERATIONS: usize = 4;

#[derive(Clone)]
pub struct Database(Arc<Inner>);

struct Inner {
    db: turso::Database,
    admission: Arc<Semaphore>,
    reads: Lane,
    writes: Lane,
    connections: RwLock<()>,
}

struct Lane {
    permits: Arc<Semaphore>,
    idle: Mutex<Vec<Connection>>,
}

impl Lane {
    fn new(size: usize) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(size)),
            idle: Mutex::new(Vec::new()),
        }
    }
}

impl Database {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        runtime::run(true, async move {
            static OPEN: OnceLock<tokio::sync::Mutex<HashMap<PathBuf, Weak<Inner>>>> =
                OnceLock::new();
            let mut registry = OPEN.get_or_init(Default::default).lock().await;
            let key = if path == Path::new(":memory:") {
                None
            } else {
                let absolute = std::path::absolute(&path)?;
                Some(if absolute.exists() {
                    absolute.canonicalize()?
                } else {
                    absolute
                        .parent()
                        .context("database parent")?
                        .canonicalize()?
                        .join(absolute.file_name().context("database filename")?)
                })
            };
            if let Some(inner) = key
                .as_ref()
                .and_then(|key| registry.get(key))
                .and_then(Weak::upgrade)
            {
                return Ok(Self(inner));
            }
            let name = path.to_str().context("database path must be UTF-8")?;
            crate::marker::record(key.as_deref().unwrap_or(&path))?;
            let db = turso::Builder::new_local(name)
                // The shared-WAL path is a Linux deployment optimization; Windows
                // falls back to regular single-process WAL with file locking.
                .experimental_multiprocess_wal(name != ":memory:" && !cfg!(windows))
                .build()
                .await
                .with_context(|| format!("open Turso database {}", path.display()))?;
            let inner = Arc::new(Inner {
                db,
                admission: Arc::new(Semaphore::new(ACTIVE_OPERATIONS)),
                reads: Lane::new(4),
                writes: Lane::new(1),
                connections: RwLock::new(()),
            });
            let database = Self(inner.clone());
            // Turso reconciles shared WAL state when a connection is opened.
            // Establish the bounded pool before any transaction can be active.
            for write in [true, false, false, false, false] {
                let connection = database.new_connection().await?;
                if !write {
                    connection.execute("PRAGMA query_only=1", ()).await?;
                }
                let lane = if write { &inner.writes } else { &inner.reads };
                lane.idle
                    .lock()
                    .expect("database idle pool poisoned")
                    .push(connection);
            }
            if let Some(key) = key {
                registry.retain(|_, db| db.strong_count() > 0);
                registry.insert(key, Arc::downgrade(&inner));
            }
            Ok(Self(inner))
        })
        .await
    }

    pub async fn open_memory() -> Result<Self> {
        Self::open(":memory:").await
    }

    /// An independent connection for diagnostics and offline maintenance.
    /// Application operations use read/write so the entire operation owns a lease.
    pub async fn connect(&self) -> Result<Connection> {
        let this = self.clone();
        runtime::run(false, async move {
            let _exclusive = this.0.connections.write().await;
            this.new_connection().await
        })
        .await
    }

    async fn new_connection(&self) -> Result<Connection> {
        let conn = self.0.db.connect()?;
        conn.busy_timeout(std::time::Duration::from_secs(30))?;
        conn.execute_batch(
            "PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA cache_size=-65536;",
        )
        .await?;
        Ok(conn)
    }

    /// A read operation sees one snapshot even when it issues multiple queries.
    pub async fn read<F, Fut, T>(&self, work: F) -> Result<T>
    where
        F: FnOnce(Connection) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T>> + Send + 'static,
        T: Send + 'static,
    {
        self.run(false, work).await
    }

    /// Serializes writers for this database; readers have separate connections.
    /// The operation owns its transaction boundaries and cannot be cancelled midway.
    pub async fn write<F, Fut, T>(&self, work: F) -> Result<T>
    where
        F: FnOnce(Connection) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T>> + Send + 'static,
        T: Send + 'static,
    {
        self.run(true, work).await
    }

    async fn run<F, Fut, T>(&self, write: bool, work: F) -> Result<T>
    where
        F: FnOnce(Connection) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T>> + Send + 'static,
        T: Send + 'static,
    {
        let queued = Instant::now();
        let this = self.clone();
        // Admit in request order up to the connection budget. Keeping this
        // queue bounded prevents fast reads from leaving every caller waiting
        // at the single writer. The admitted reads and write still run together.
        let admission = this
            .0
            .admission
            .clone()
            .acquire_owned()
            .await
            .context("database admission closed")?;
        runtime::run(write, async move {
            let _admission = admission;
            let lane = if write { &this.0.writes } else { &this.0.reads };
            let permit = lane
                .permits
                .clone()
                .acquire_owned()
                .await
                .context("database pool closed")?;
            this.execute(write, permit, queued, work).await
        })
        .await
    }

    async fn execute<F, Fut, T>(
        &self,
        write: bool,
        _permit: OwnedSemaphorePermit,
        queued: Instant,
        work: F,
    ) -> Result<T>
    where
        F: FnOnce(Connection) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let started = Instant::now();
        let queue_us = queued.elapsed().as_micros() as u64;
        let lane = if write { &self.0.writes } else { &self.0.reads };
        let mut access = self.0.connections.read().await;
        let cached = lane.idle.lock().expect("database idle pool poisoned").pop();
        let conn = match cached {
            Some(conn) => conn,
            None => {
                drop(access);
                let exclusive = self.0.connections.write().await;
                let conn = self.new_connection().await?;
                if !write {
                    conn.execute("PRAGMA query_only=1", ()).await?;
                }
                access = RwLockWriteGuard::downgrade(exclusive);
                conn
            }
        };
        if !write {
            conn.prepare_cached("BEGIN").await?.execute(()).await?;
        }
        let result = work(conn.clone()).await;
        // Keep the shared guard through transaction cleanup, including COMMIT.
        let _access = access;
        // Turso defers Transaction::drop rollback until the next operation.
        // Flush that rollback before inspecting autocommit or returning a lease.
        if result.is_err() {
            if let Err(error) = conn.execute_batch("").await {
                tracing::warn!(%error, "discarding connection after failed rollback");
                return result;
            }
        }
        let finish = if !write && result.is_ok() {
            "COMMIT"
        } else {
            "ROLLBACK"
        };
        // Successful writes must have closed their own transaction. An unexpected
        // active write transaction is rolled back and rejected, never pooled dirty.
        let active = !conn.is_autocommit()?;
        let cleanup = if active {
            match conn.prepare_cached(finish).await {
                Ok(mut statement) => statement.execute(()).await.map(|_| ()),
                Err(error) => Err(error),
            }
        } else {
            Ok(())
        };
        let incomplete_write = write && active && result.is_ok();
        if cleanup.is_ok() && !incomplete_write {
            lane.idle
                .lock()
                .expect("database idle pool poisoned")
                .push(conn);
        }
        tracing::debug!(
            write,
            queue_us,
            execution_us = started.elapsed().as_micros() as u64,
            success = result.is_ok() && cleanup.is_ok(),
            "database operation finished"
        );
        if let Err(error) = cleanup {
            tracing::warn!(%error, "discarding connection after transaction cleanup failure");
            return result.and(Err(error).context("finish database operation"));
        }
        anyhow::ensure!(
            !incomplete_write,
            "write operation left a transaction open; rolled back"
        );
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancelled_caller_does_not_remove_an_admitted_queued_write() {
        let database = Database::open_memory().await.unwrap();
        let held = database
            .0
            .writes
            .permits
            .clone()
            .acquire_owned()
            .await
            .unwrap();
        let copy = database.clone();
        let task = tokio::spawn(async move {
            copy.write(|connection| async move {
                connection
                    .execute("CREATE TABLE accepted(id INTEGER)", ())
                    .await?;
                Ok(())
            })
            .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while database.0.admission.available_permits() == ACTIVE_OPERATIONS {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        task.abort();
        drop(held);
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while database.0.admission.available_permits() != ACTIVE_OPERATIONS {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        database
            .read(|connection| async move {
                let mut rows = connection
                    .query("SELECT count(*) FROM accepted", ())
                    .await?;
                assert_eq!(rows.next().await?.unwrap().get::<i64>(0)?, 0);
                Ok(())
            })
            .await
            .unwrap();
    }
}
