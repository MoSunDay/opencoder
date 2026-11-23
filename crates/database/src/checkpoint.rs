//! Periodic passive WAL checkpoints keep write-ahead logs bounded while the
//! pool stays online. One independent diagnostic connection per attempt runs
//! `PRAGMA wal_checkpoint(PASSIVE)` under a short busy timeout: contention
//! means skip without blocking, and `wal_autocheckpoint` still bounds the WAL
//! between passes. The strict TRUNCATE variant stays a CLI-only operation.

use anyhow::{Context, Result};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::Duration,
};

/// Never wait longer than this for a contended checkpoint attempt: the
/// periodic pass skips instead of blocking application traffic.
const BUSY_TIMEOUT: Duration = Duration::from_secs(2);

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
    /// A concurrent reader/writer holds the WAL; skipped without blocking.
    Busy,
    /// WAL merged back into the main database file.
    Merged {
        before_bytes: u64,
        after_bytes: u64,
        checkpointed_pages: i64,
    },
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

/// WAL companion path as laid out by the engine: `{database}-wal`.
pub fn wal_path(db: &Path) -> PathBuf {
    PathBuf::from(format!("{}-wal", db.display()))
}

/// Current WAL size in bytes; an absent WAL counts as zero.
pub fn wal_size(db: &Path) -> u64 {
    std::fs::metadata(wal_path(db))
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

/// Open ONE independent writable diagnostic connection and run a PASSIVE
/// checkpoint. Never waits longer than a short busy timeout: contention
/// means the attempt reports busy, and `wal_autocheckpoint` still bounds
/// the WAL.
pub async fn checkpoint_passive(db: &Path) -> Result<CheckpointStats> {
    let path = db.to_path_buf();
    crate::runtime::run(true, async move {
        let name = path.to_str().context("database path must be UTF-8")?;
        let database = turso::Builder::new_local(name)
            .experimental_multiprocess_wal(true)
            .build()
            .await
            .with_context(|| format!("open checkpoint connection {}", path.display()))?;
        let connection = database.connect()?;
        connection.busy_timeout(BUSY_TIMEOUT)?;
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
        drop(rows);
        drop(connection);
        drop(database);
        stats.context("wal_checkpoint returned no result row")
    })
    .await
}

/// One gated pass: below the threshold nothing is done; otherwise a passive
/// checkpoint runs and the WAL size delta is reported. This is the core
/// testable unit.
pub async fn run_once(db: &Path, threshold_bytes: u64) -> Result<Outcome> {
    let before_bytes = wal_size(db);
    if before_bytes < threshold_bytes {
        return Ok(Outcome::Skipped {
            wal_bytes: before_bytes,
        });
    }
    let stats = checkpoint_passive(db).await?;
    if stats.busy {
        // The WAL still holds its frames, so the byte gate stays meaningful
        // for the next pass; `wal_autocheckpoint` bounds growth meanwhile.
        return Ok(Outcome::Busy);
    }
    let after_bytes = wal_size(db);
    Ok(Outcome::Merged {
        before_bytes,
        after_bytes,
        checkpointed_pages: stats.checkpointed_pages,
    })
}

/// Start the per-path periodic loop (process lifetime, no explicit stop):
/// a registry keyed by canonical path prevents a double start when
/// `Database::open` re-opens the same file.
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
            match run_once(&db, policy.wal_threshold_bytes).await {
                Ok(Outcome::Merged {
                    before_bytes,
                    after_bytes,
                    ..
                }) => tracing::info!(
                    path = %db.display(),
                    before = before_bytes,
                    after = after_bytes,
                    "periodic wal checkpoint merged"
                ),
                Ok(Outcome::Busy) => tracing::debug!(
                    path = %db.display(),
                    "periodic wal checkpoint busy; skipped without blocking"
                ),
                Ok(Outcome::Skipped { .. }) => {}
                Err(error) => tracing::warn!(
                    path = %db.display(),
                    %error,
                    "periodic wal checkpoint failed"
                ),
            }
        }
    });
}
