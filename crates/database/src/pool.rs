use crate::engine;
use crate::lease::ConnectionLease;
use crate::maintenance_lock::MaintenanceLease;
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

/// Why the database is being opened. A writer holds a shared maintenance
/// lease ([`crate::maintenance_lock`]) on the file-backed database for its
/// whole lifetime: any number of writers, here or in any other process, open
/// the same file through the engine's multi-process WAL and commit beside each
/// other, while offline maintenance fails fast beside them with
/// [`crate::maintenance_lock::WritersActive`]. A reader takes no lease, opens
/// the engine read-only, sees every writer's committed WAL frames and never
/// bootstraps, migrates, checkpoints or writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Intent {
    Writer,
    Reader,
}

#[derive(Clone)]
pub struct Database(Arc<Inner>);

/// Shared by the pool registry and the checkpoint registry: the latter keeps
/// a weak reference, so a handle stays eligible exactly as long as the pool
/// that published it is alive.
pub(crate) struct Inner {
    db: turso::Database,
    intent: Intent,
    path: PathBuf,
    admission: Arc<Semaphore>,
    reads: Lane,
    writes: Lane,
    connections: RwLock<()>,
    /// The shared maintenance lease, held for the whole writable lifetime and
    /// dropped with the pool, never by the checkpoint registry. Declared last:
    /// fields drop in declaration order, so the lease is released only after
    /// the engine and every pooled connection are gone. Nothing reads it:
    /// holding it for exactly the pool's lifetime is its whole purpose -- it
    /// is what tells offline maintenance that this writer is alive. A
    /// [`ConnectionLease`] that escaped the pool keeps that lifetime going
    /// until its last clone drops. `None` for a reader and for an in-memory
    /// database.
    #[allow(dead_code)]
    lease: Option<MaintenanceLease>,
}

/// Symlink chain limit for [`canonical_key`], matching the usual kernel one.
const MAX_SYMLINK_FOLLOWS: u8 = 40;

/// Registry key for `path`, and the one path every file-backed artifact is
/// derived from: the engine open, the maintenance lease, the engine marker
/// and the WAL companion. Opening through a symlink must resolve to the same
/// key as opening the file it names, otherwise the lease and the marker sit
/// next to one path while the engine writes its WAL next to the other -- and
/// offline maintenance of the one would never see the writer holding a lease
/// on the other.
///
/// An existing file is canonicalized. A file still to be created resolves
/// its parent and follows a symlink in the final component even when the
/// target does not exist yet, so the key names the file that will exist
/// rather than the link pointing at it.
fn canonical_key(path: &Path) -> Result<PathBuf> {
    let mut candidate = std::path::absolute(path)?;
    for _ in 0..MAX_SYMLINK_FOLLOWS {
        if candidate.exists() {
            return Ok(candidate.canonicalize()?);
        }
        let parent = candidate
            .parent()
            .context("database parent")?
            .canonicalize()
            .with_context(|| format!("resolve database directory {}", candidate.display()))?;
        let name = candidate.file_name().context("database filename")?;
        candidate = parent.join(name);
        let is_symlink = std::fs::symlink_metadata(&candidate)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false);
        if !is_symlink {
            return Ok(candidate);
        }
        let target = std::fs::read_link(&candidate)
            .with_context(|| format!("resolve database symlink {}", candidate.display()))?;
        candidate = if target.is_absolute() {
            target
        } else {
            parent.join(target)
        };
    }
    anyhow::bail!(
        "database path resolves through more than {MAX_SYMLINK_FOLLOWS} symlinks: {}",
        path.display()
    )
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

/// Open pools per canonical path and intent; a dropped pool leaves a dead
/// weak entry behind, which the next open of the same key prunes.
type Registry = HashMap<(PathBuf, Intent), Weak<Inner>>;

impl Database {
    /// Open `path` for writing (the default intent). The shared maintenance
    /// lease is taken before the engine opens the file and held until the
    /// pool is dropped, so the pool is one more writer beside every other
    /// writer of the file, in this or any other process; a repeated open while
    /// the pool is alive reuses that pool. The open waits out an exclusive
    /// lease that is rewriting the file and fails with
    /// [`crate::maintenance_lock::MaintenanceActive`] once that wait exceeds
    /// its budget.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(path, Intent::Writer).await
    }

    /// Open `path` read-only: no lease, and `write` is rejected. Committed
    /// WAL frames of every live writer, in this or any other process, stay
    /// visible through the shared-WAL path.
    pub async fn open_reader(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(path, Intent::Reader).await
    }

    /// Open `path` with an explicit `intent`. The pool registry is keyed by
    /// `(canonical path, intent)`, so a reader and a writer on one file
    /// coexist inside a process and a repeated open of the same file reuses
    /// the pool that already serves it.
    pub async fn open_with(path: impl AsRef<Path>, intent: Intent) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        runtime::run(true, async move {
            static OPEN: OnceLock<tokio::sync::Mutex<Registry>> = OnceLock::new();
            let mut registry = OPEN.get_or_init(Default::default).lock().await;
            let key = if path == Path::new(":memory:") {
                None
            } else {
                Some(canonical_key(&path)?)
            };
            let existing = key
                .as_ref()
                .and_then(|key| registry.get(&(key.clone(), intent)))
                .and_then(Weak::upgrade);
            let inner = if let Some(inner) = existing.as_ref() {
                Arc::clone(inner)
            } else {
                // The engine opens the canonical path too: the WAL and the
                // shared-memory companions it derives from that path must sit
                // next to the file the maintenance lease and the marker name.
                // A path the engine cannot open is refused here, before the
                // marker is recorded and before the lease is taken.
                let name = crate::engine::engine_name(
                    key.as_ref().map_or(path.as_path(), PathBuf::as_path),
                )?;
                match (&key, intent) {
                    (Some(key), Intent::Writer) => crate::marker::record(key)?,
                    (Some(key), Intent::Reader) => crate::marker::verify(key)?,
                    _ => {}
                }
                // A writer leases the file before the engine opens it, so a
                // TRUNCATE checkpoint or a writable bridge running concurrently
                // fails fast instead of rewriting the file under it. The first
                // open of a file (the one that initializes the WAL coordination
                // companion) runs under the exclusive lease, so no two writers
                // ever initialise the same file. The lease comes back behind
                // the engine and is dropped only after it.
                let (db, lease) = match (&key, intent) {
                    (Some(key), Intent::Writer) => {
                        let (db, held) = engine::open_writer(key, &path, || async {}).await?;
                        (db, Some(held))
                    }
                    _ => (
                        engine::open(name, intent == Intent::Reader, &path).await?,
                        None,
                    ),
                };
                let inner = Arc::new(Inner {
                    db,
                    intent,
                    path: key.clone().unwrap_or_else(|| path.clone()),
                    lease,
                    admission: Arc::new(Semaphore::new(ACTIVE_OPERATIONS)),
                    reads: Lane::new(4),
                    writes: Lane::new(1),
                    connections: RwLock::new(()),
                });
                let database = Self(inner.clone());
                // Turso reconciles shared WAL state when a connection is opened.
                // Establish the bounded pool before any transaction can be active.
                // Readers get read lanes only: nothing may ever write.
                let lanes: &[bool] = if intent == Intent::Writer {
                    &[true, false, false, false, false]
                } else {
                    &[false, false, false, false]
                };
                for write in lanes.iter().copied() {
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
                if let Some(key) = &key {
                    registry.retain(|_, db| db.strong_count() > 0);
                    registry.insert((key.clone(), intent), Arc::downgrade(&inner));
                }
                inner
            };
            // Publish the pool's own handle so the periodic loop and
            // `handle_for` reuse it instead of reconnecting every pass. Only
            // a writer pool registers a checkpoint source: a checkpoint writes
            // and a reader never writes, and the loop checks on every pass
            // that the pool the handle -- and with it the shared lease -- is
            // bound to is still alive.
            // A freshly built pool primes the dedicated checkpoint connection
            // now, while no application operation can be in flight; a registry
            // hit keeps the handle that is already primed for this engine.
            if let (Some(key), Intent::Writer) = (&key, intent) {
                let handle = crate::checkpoint::CheckpointHandle::new(
                    key.clone(),
                    inner.db.clone(),
                    Arc::downgrade(&inner),
                );
                if existing.is_none() {
                    if let Err(error) = handle.prime().await {
                        tracing::warn!(
                            path = %key.display(),
                            %error,
                            "checkpoint connection not primed; the first pass connects lazily"
                        );
                    }
                    crate::checkpoint::register(handle);
                } else {
                    crate::checkpoint::register_if_absent(handle);
                }
                // Every file-backed writer pool runs the periodic passive WAL
                // checkpoint loop (memory databases have no WAL file). The
                // pass runs on the pool's own handle, so it happens only
                // while this pool is alive; passes of several processes may
                // overlap, the engine's WAL locks serialise them and a busy
                // pass is skipped.
                crate::checkpoint::start_periodic(
                    key.clone(),
                    crate::checkpoint::CheckpointPolicy::default(),
                );
            }
            Ok(Self(inner))
        })
        .await
    }

    /// Intent this handle was opened with.
    pub fn intent(&self) -> Intent {
        self.0.intent
    }

    pub async fn open_memory() -> Result<Self> {
        Self::open(":memory:").await
    }

    /// An independent connection for diagnostics and offline maintenance.
    /// Application operations use read/write so the entire operation owns
    /// its transaction boundaries. The connection carries a reference to
    /// this pool, so a writer's shared maintenance lease outlives the pool
    /// for exactly as long as one of its connections is alive.
    pub async fn connect(&self) -> Result<ConnectionLease> {
        let this = self.clone();
        runtime::run(false, async move {
            let _exclusive = this.0.connections.write().await;
            let connection = this.new_connection().await?;
            Ok(ConnectionLease::new(Arc::clone(&this.0), connection))
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
        if self.0.intent == Intent::Reader {
            // Belt and braces with the engine-level read-only flag: no
            // reader connection may ever issue a write statement.
            conn.execute("PRAGMA query_only=1", ()).await?;
        }
        Ok(conn)
    }

    /// A read operation sees one snapshot even when it issues multiple queries.
    pub async fn read<F, Fut, T>(&self, work: F) -> Result<T>
    where
        F: FnOnce(ConnectionLease) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T>> + Send + 'static,
        T: Send + 'static,
    {
        self.run(false, work).await
    }

    /// Serializes this pool's writers; readers have separate connections.
    /// The operation owns its transaction boundaries and cannot be cancelled midway.
    /// A handle opened with [`Intent::Reader`] rejects writes before touching
    /// the engine.
    pub async fn write<F, Fut, T>(&self, work: F) -> Result<T>
    where
        F: FnOnce(ConnectionLease) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T>> + Send + 'static,
        T: Send + 'static,
    {
        anyhow::ensure!(
            self.0.intent == Intent::Writer,
            "database {} was opened read-only; writes need a writer open",
            self.0.path.display()
        );
        self.run(true, work).await
    }

    async fn run<F, Fut, T>(&self, write: bool, work: F) -> Result<T>
    where
        F: FnOnce(ConnectionLease) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T>> + Send + 'static,
        T: Send + 'static,
    {
        let queued = Instant::now();
        let this = self.clone();
        // Admit in request order up to the connection budget. Keeping this
        // queue bounded prevents fast reads from leaving every caller waiting
        // at the write lane. The admitted reads and write still run together.
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
        F: FnOnce(ConnectionLease) -> Fut,
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
        let result = work(ConnectionLease::new(Arc::clone(&self.0), conn.clone())).await;
        // Keep the shared guard through transaction cleanup, including COMMIT.
        let _access = access;
        // Turso defers Transaction::drop rollback until the next operation.
        // Flush that rollback before inspecting autocommit or pooling the connection.
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
