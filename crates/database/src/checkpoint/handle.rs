//! The resident checkpoint source: one engine, one dedicated connection and
//! the binding to the pool it may checkpoint for.

use super::{prune, BUSY_TIMEOUT};
use crate::maintenance_lock::MaintenanceLease;
use anyhow::{Context, Result};
use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Weak,
    },
};
use tokio::sync::Mutex as AsyncMutex;

/// Why a handle may not checkpoint: the pool that published it is gone.
pub(crate) fn pool_is_gone(handle: &CheckpointHandle) -> anyhow::Error {
    anyhow::anyhow!(
        "checkpoint handle for {} is bound to a pool that is gone; it may not touch the WAL",
        handle.path().display()
    )
}

/// How a handle is bound to what keeps it alive.
enum Ownership {
    /// A pool published this handle. Only a weak reference is kept, so the
    /// pool's drop disarms the handle at once and its retained engine may
    /// never checkpoint again.
    Pooled(Weak<crate::pool::Inner>),
    /// A standalone handle opened its own engine under the database's shared
    /// maintenance lease and releases both together, so its writable engine
    /// never outlives the lease that tells offline maintenance this writer is
    /// alive. Nothing reads the lease: holding it for exactly the engine's
    /// lifetime is its whole purpose.
    Standalone(#[allow(dead_code)] MaintenanceLease),
}

/// Resident checkpoint source bound to an already-open database handle AND to
/// the pool that owned the database when the handle was built.
///
/// Reusing the pool's `turso::Database` alone is not enough: the engine runs
/// its shared-WAL authority repair on `Database::connect()` when the file was
/// opened through the disk-scan path, and that repair may clear sibling
/// reader slots mid-transaction. The handle therefore owns one dedicated
/// connection, opened while the database is still quiescent
/// ([`CheckpointHandle::prime`]) and reused by every pass. A failed pass
/// discards that connection (its pager may keep the failed page view) and the
/// next pass opens a fresh one -- the only `connect()` after start-up,
/// accepted because it follows a failure rather than a timer.
pub struct CheckpointHandle {
    database: turso::Database,
    path: PathBuf,
    /// The dedicated checkpoint connection: `None` until primed and again
    /// after a failed pass discarded it. The lock also serializes passes.
    connection: AsyncMutex<Option<turso::Connection>>,
    /// `connect()` calls made so far: `1` while every pass succeeds, plus
    /// one for each connection discarded after a failed pass.
    connection_opens: AtomicUsize,
    /// Declared last: fields drop in declaration order, so a standalone
    /// handle releases its lease only after its engine and connection are gone.
    ownership: Ownership,
}

/// The three result columns of `PRAGMA wal_checkpoint`: whether the attempt
/// hit contention, how many pages the WAL spans, and how many were merged
/// back into the main database file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckpointStats {
    pub busy: bool,
    pub log_pages: i64,
    pub checkpointed_pages: i64,
}

impl CheckpointHandle {
    /// Bind an already-open engine to the pool that owns it. The handle keeps
    /// only a weak reference, so it stays eligible exactly as long as that
    /// pool is alive.
    pub(crate) fn new(
        path: PathBuf,
        database: turso::Database,
        pool: Weak<crate::pool::Inner>,
    ) -> Self {
        Self {
            database,
            path,
            ownership: Ownership::Pooled(pool),
            connection: AsyncMutex::new(None),
            connection_opens: AtomicUsize::new(0),
        }
    }

    /// Bind an engine to the maintenance lease it was opened under, so both
    /// are released together. Used for a standalone pass, which is never
    /// registered.
    pub(crate) fn standalone(
        path: PathBuf,
        database: turso::Database,
        lease: MaintenanceLease,
    ) -> Self {
        Self {
            database,
            path,
            ownership: Ownership::Standalone(lease),
            connection: AsyncMutex::new(None),
            connection_opens: AtomicUsize::new(0),
        }
    }

    /// Database path this handle checkpoints.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Dedicated connections opened so far: one, plus one per discarded connection.
    pub fn connection_opens(&self) -> usize {
        self.connection_opens.load(Ordering::Relaxed)
    }

    /// Whether what this handle is bound to is still alive: the pool that
    /// published it, or the standalone lease it owns. `false` once that pool
    /// is gone: the handle and its engine may still be resident, but they may
    /// no longer touch the WAL.
    pub fn owner_alive(&self) -> bool {
        match &self.ownership {
            Ownership::Pooled(pool) => pool.upgrade().is_some(),
            Ownership::Standalone(_) => true,
        }
    }

    /// Open the dedicated connection now. Pools prime while no application
    /// operation can be in flight, so the one `connect()` of this handle
    /// never races a sibling transaction; a pass primes lazily if this
    /// was skipped or failed. A handle whose pool is gone refuses:
    /// `connect()` is the very call this module must not repeat on a stale
    /// engine.
    pub async fn prime(&self) -> Result<()> {
        anyhow::ensure!(self.owner_alive(), "{}", pool_is_gone(self));
        let mut connection = self.connection.lock().await;
        if connection.is_none() {
            *connection = Some(self.open_connection()?);
        }
        Ok(())
    }

    /// Every `connect()` this handle performs: the pool's priming open and
    /// the re-open after a failed pass, each with the bounded busy timeout.
    fn open_connection(&self) -> Result<turso::Connection> {
        let connection = self
            .database
            .connect()
            .with_context(|| format!("open checkpoint connection {}", self.path.display()))?;
        connection.busy_timeout(BUSY_TIMEOUT)?;
        self.connection_opens.fetch_add(1, Ordering::Relaxed);
        Ok(connection)
    }
}

/// Run `pass` on the handle's dedicated connection. A failed pass discards
/// the connection right away (its pager may keep the failed page view; the
/// pool likewise never returns a connection that failed back to a lane) and
/// the next pass opens a fresh one. A busy result is not a failure: the
/// connection stays. The pass keeps its own clone of the shared handle, so
/// engine -- and for a standalone handle its maintenance lease -- stays alive
/// for the whole pass even if the pool that published the handle is dropped;
/// for as long as it runs it is one of the database's writers.
async fn run_pass<P, Fut>(handle: &Arc<CheckpointHandle>, pass: P) -> Result<CheckpointStats>
where
    P: FnOnce(turso::Connection) -> Fut,
    P: Send + 'static,
    Fut: Future<Output = Result<CheckpointStats>> + Send + 'static,
{
    let handle = Arc::clone(handle);
    crate::runtime::run(true, async move {
        let mut slot = handle.connection.lock().await;
        if slot.is_none() {
            *slot = Some(handle.open_connection()?);
        }
        let connection = slot
            .as_ref()
            .expect("checkpoint connection initialized above");
        let result = pass(connection.clone()).await;
        if result.is_err() {
            *slot = None;
        }
        result
    })
    .await
}

/// Run a PASSIVE checkpoint on the handle's dedicated connection, opening it
/// only when the pool could not prime it or a failed pass discards it.
/// Never waits longer than a short busy timeout: contention means the
/// attempt reports busy, and `wal_autocheckpoint` still bounds the WAL. The
/// connection lock serializes concurrent passes.
///
/// The handle's pool must still be alive: a handle bound to a dropped pool
/// is refused before any statement runs, in release builds as well as debug.
pub async fn checkpoint_passive(handle: &Arc<CheckpointHandle>) -> Result<CheckpointStats> {
    anyhow::ensure!(handle.owner_alive(), "{}", pool_is_gone(handle));
    run_pass(handle, wal_checkpoint_passive).await
}

/// The PASSIVE checkpoint statement on `connection`, parsed into stats.
async fn wal_checkpoint_passive(connection: turso::Connection) -> Result<CheckpointStats> {
    let mut rows = connection
        .query("PRAGMA wal_checkpoint(PASSIVE)", ())
        .await?;
    let mut stats: Option<CheckpointStats> = None;
    while let Some(row) = rows.next().await? {
        anyhow::ensure!(
            stats.is_none(),
            "wal_checkpoint returned more than one result row"
        );
        stats = Some(CheckpointStats {
            busy: row.get::<i64>(0)? != 0,
            log_pages: row.get::<i64>(1)?,
            checkpointed_pages: row.get::<i64>(2)?,
        });
    }
    stats.context("wal_checkpoint returned no result row")
}

/// One gated pass on `handle`: below the threshold nothing is done; otherwise
/// a passive checkpoint runs and the WAL size delta is reported. This is the
/// core testable unit. Eligibility comes from the handle itself, so a caller
/// can never vouch for a handle whose pool is gone.
pub async fn run_once_handle(
    handle: &Arc<CheckpointHandle>,
    threshold_bytes: u64,
) -> Result<super::Outcome> {
    anyhow::ensure!(handle.owner_alive(), "{}", pool_is_gone(handle));
    let before_bytes = super::wal_size(handle.path());
    if before_bytes < threshold_bytes {
        return Ok(super::Outcome::Skipped {
            wal_bytes: before_bytes,
        });
    }
    let stats = run_pass(handle, wal_checkpoint_passive).await?;
    if stats.busy {
        // The WAL still holds its frames, so the byte gate stays meaningful
        // for the next pass; `wal_autocheckpoint` bounds growth meanwhile.
        return Ok(super::Outcome::Busy);
    }
    let after_bytes = super::wal_size(handle.path());
    Ok(super::Outcome::Merged {
        before_bytes,
        after_bytes,
        checkpointed_pages: stats.checkpointed_pages,
    })
}

/// Drop the registry entry of `handle` when its pool is gone. Called by the
/// periodic loop before it decides to wait for the next pass.
pub(crate) fn prune_if_orphaned(handle: &Arc<CheckpointHandle>) -> bool {
    if handle.owner_alive() {
        return false;
    }
    prune(handle);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A primed standalone handle on a fresh file: exactly one open so far.
    async fn primed_handle(directory: &Path) -> Arc<CheckpointHandle> {
        let path = directory.join("passes.db");
        let name = path.to_str().expect("utf-8 temp path").to_owned();
        let database = crate::runtime::run(true, async move {
            turso::Builder::new_local(&name)
                .experimental_multiprocess_wal(true)
                .build()
                .await
                .context("build checkpoint test database")
        })
        .await
        .expect("open checkpoint test database");
        // A standalone handle owns the shared lease its engine runs under.
        let lease =
            crate::maintenance_lock::acquire_shared(&path).expect("lease the test database");
        let handle = Arc::new(CheckpointHandle::standalone(path, database, lease));
        handle.prime().await.expect("prime checkpoint connection");
        handle
    }

    /// A handle is bound to its pool, not to the path: once that pool is
    /// dropped the handle stays refused even though the path is opened again
    /// by a new pool, whose own handle is the eligible one.
    #[tokio::test]
    async fn a_handle_dies_with_its_pool_and_never_qualifies_again() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pool.db");
        let pool = crate::Database::open(&path).await.unwrap();
        let key = path.canonicalize().unwrap();
        let handle = super::super::handle_for(&key).await.unwrap();
        let opens = handle.connection_opens();
        assert!(
            handle.owner_alive(),
            "the pool publishes an eligible handle"
        );

        drop(pool);
        assert!(
            !handle.owner_alive(),
            "the pool's drop must disarm the handle"
        );
        assert!(
            prune_if_orphaned(&handle),
            "the loop prunes the dead handle"
        );
        assert_eq!(
            super::super::periodic_pass(&key, 0).await.unwrap(),
            None,
            "nothing is eligible after the prune"
        );
        for refused in [
            handle.prime().await.unwrap_err(),
            checkpoint_passive(&handle).await.unwrap_err(),
            run_once_handle(&handle, 0).await.unwrap_err(),
        ] {
            assert_eq!(refused.to_string(), pool_is_gone(&handle).to_string());
        }
        assert_eq!(
            opens,
            handle.connection_opens(),
            "a refused call never connects"
        );

        // A re-opened pool publishes its own handle; the superseded one stays
        // refused although the path is owned again.
        let second = crate::Database::open(&path).await.unwrap();
        let current = super::super::handle_for(&key).await.unwrap();
        assert!(!Arc::ptr_eq(&handle, &current));
        assert!(!handle.owner_alive());
        assert!(current.owner_alive());
        assert!(run_once_handle(&current, u64::MAX).await.is_ok());
        drop(second);
        assert!(
            !current.owner_alive(),
            "the new handle dies with its pool too"
        );
    }

    #[tokio::test]
    async fn failed_pass_discards_the_dedicated_connection() {
        let directory = tempfile::tempdir().unwrap();
        let handle = primed_handle(directory.path()).await;
        assert_eq!(handle.connection_opens(), 1);

        let error = run_pass(&handle, |_| async {
            Err(anyhow::anyhow!("injected checkpoint failure"))
        })
        .await
        .expect_err("a failed pass surfaces its error");
        assert_eq!(error.to_string(), "injected checkpoint failure");
        assert!(
            handle.connection.lock().await.is_none(),
            "the failed pass must discard the connection"
        );

        let stats = checkpoint_passive(&handle)
            .await
            .expect("the next pass opens a fresh connection");
        assert!(!stats.busy);
        assert_eq!(
            handle.connection_opens(),
            2,
            "one fresh open after the discard"
        );
        checkpoint_passive(&handle)
            .await
            .expect("later passes reuse the fresh connection");
        assert_eq!(
            handle.connection_opens(),
            2,
            "no further opens while passes succeed"
        );
    }
}
