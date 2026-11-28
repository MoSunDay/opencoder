//! The periodic passive checkpoint loop: one per database path and process.

use super::{handle::prune_if_orphaned, run_once_handle};
use anyhow::Result;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::Duration,
};

/// Cadence and size gate for the periodic checkpoint loop.
#[derive(Clone, Copy, Debug)]
pub struct CheckpointPolicy {
    pub interval: Duration,
    pub wal_threshold_bytes: u64,
}

impl Default for CheckpointPolicy {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(60),
            wal_threshold_bytes: 1024 * 1024,
        }
    }
}

/// One gated checkpoint pass outcome (observable for logs and tests).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Below the size gate; nothing done.
    Skipped { wal_bytes: u64 },
    /// A concurrent reader/writer or another process's checkpointer holds
    /// the WAL; skipped without blocking.
    Busy,
    /// WAL merged back into the main database file.
    Merged {
        before_bytes: u64,
        after_bytes: u64,
        checkpointed_pages: i64,
    },
}

/// Start the per-path periodic loop (process lifetime, no explicit stop):
/// a registry keyed by canonical path prevents a double start when
/// `Database::open` re-opens the same file. Only started for writer pools.
pub fn start_periodic(db: PathBuf, policy: CheckpointPolicy) {
    static STARTED: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    let started = STARTED.get_or_init(Default::default);
    if !started
        .lock()
        .expect("checkpoint start registry poisoned")
        .insert(db.clone())
    {
        return;
    }
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(policy.interval).await;
            match periodic_pass(&db, policy.wal_threshold_bytes).await {
                Ok(None) => {}
                Ok(Some(Outcome::Merged {
                    before_bytes,
                    after_bytes,
                    ..
                })) => tracing::info!(
                    path = %db.display(),
                    before = before_bytes,
                    after = after_bytes,
                    "periodic wal checkpoint merged"
                ),
                Ok(Some(Outcome::Busy)) => tracing::debug!(
                    path = %db.display(),
                    "periodic wal checkpoint busy; skipped without blocking"
                ),
                Ok(Some(Outcome::Skipped { .. })) => {}
                Err(error) => tracing::warn!(
                    path = %db.display(),
                    %error,
                    "periodic wal checkpoint failed"
                ),
            }
        }
    });
}

/// One pass of the periodic loop for `db`, and the unit its eligibility gate
/// is tested through: `Ok(None)` means nothing was eligible and nothing ran.
///
/// The CURRENT resident handle is looked up every pass, so the loop follows a
/// re-registered pool handle after the pool re-opens the file. That handle
/// is used only while the pool it is bound to is still alive; a retained
/// handle whose pool is gone is pruned and never checkpointed. The loop
/// itself takes no lease and opens no engine: without an eligible handle
/// there is nothing to do, so it waits for the next pass instead. Only a
/// process with a live writer pool of the file has a resident handle, and a
/// pass that finds the engine's checkpointer lock taken -- another process
/// may be checkpointing the same file -- reports busy and is skipped.
pub async fn periodic_pass(db: &Path, threshold_bytes: u64) -> Result<Option<Outcome>> {
    let Some(handle) = super::registered(db) else {
        return Ok(None);
    };
    if prune_if_orphaned(&handle) {
        return Ok(None);
    }
    run_once_handle(&handle, threshold_bytes).await.map(Some)
}
