//! Engine opens for file-backed databases.
//!
//! Writers never exclude each other: every writable open -- a pool, a
//! standalone checkpoint pass, in this process or any other -- joins the same
//! multi-process shared WAL and commits beside the others, serialised by the
//! engine's own WAL writer lock. What an open must still get right is who else
//! is touching the file meanwhile:
//!
//! - Offline maintenance (the TRUNCATE checkpoint, the writable maintenance
//!   bridge) holds the database's EXCLUSIVE maintenance lease
//!   ([`crate::maintenance_lock`]) while it rewrites the file. A writer
//!   therefore takes a SHARED lease before the engine opens and keeps it for
//!   the engine's whole lifetime: maintenance fails fast with
//!   [`maintenance_lock::WritersActive`] beside it, and a writer that arrives
//!   beside maintenance waits the exclusive lease out instead of opening a
//!   file that is being truncated underneath it.
//! - The FIRST opener of a file creates its WAL coordination companion
//!   (`{db}-tshm`) empty, sizes it and initialises it. That one open runs
//!   under the exclusive lease and downgrades to shared as soon as the engine
//!   is built, so two processes racing a first open cannot both initialise the
//!   file, and maintenance cannot slip in halfway. The loser of that race
//!   re-checks the companion and joins as an ordinary shared writer.
//! - A reader takes no lease: it opens the file read-only and waits out a
//!   still-short coordination companion with a bounded retry, the one state a
//!   concurrent first open can show it.
//!
//! An in-memory database takes none of this: it has no file to lease.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::maintenance_lock::{self, MaintenanceLease, Mode};

/// Budget for waiting out a concurrent first open of the same file.
const COORDINATION_INIT_BUDGET: Duration = Duration::from_secs(10);
/// Pause between two attempts to open a file another process initializes.
const COORDINATION_INIT_PAUSE: Duration = Duration::from_millis(20);
/// Budget for acquiring a writer's lease: offline maintenance and a concurrent
/// first open are both short, and a writer that outlives this budget fails
/// with the lease conflict instead of queueing behind them forever.
const LEASE_WAIT_BUDGET: Duration = Duration::from_secs(120);
/// Pause between two attempts to lease a database another process is working on.
const LEASE_WAIT_PAUSE: Duration = Duration::from_millis(20);

/// The engine takes a string path, so a path that is not UTF-8 can never be
/// opened; every entry point that takes a lease or creates a companion for a
/// database refuses such a path here, first, so the refusal leaves no lease
/// file, marker or engine companion behind.
pub(crate) fn engine_name(path: &Path) -> Result<&str> {
    path.to_str()
        .with_context(|| format!("database path must be UTF-8: {}", path.display()))
}

/// The engine's WAL coordination companion for `db`: `{db}-tshm`.
///
/// Its absence is what makes an open the first one: the engine creates the
/// companion on the first writable open and never deletes it, so only a
/// brand-new file, a database last written by an older engine, or a bare copy
/// of the `.db` alone has none yet. The suffix is appended to the path's own
/// OS string, never to a rendered form of it.
pub(crate) fn coordination_path(db: &Path) -> PathBuf {
    let mut name = db.as_os_str().to_os_string();
    name.push("-tshm");
    PathBuf::from(name)
}

/// Whether the engine refused the file because another process is still
/// initializing its WAL coordination companion: the engine creates that file
/// empty and writes its header afterwards, so an opener arriving in between
/// reports the short file as corrupt. That state lasts only as long as the
/// first open.
pub(crate) fn is_coordination_file_uninitialized(error: &turso::Error) -> bool {
    matches!(error, turso::Error::Corrupt(message) if message.starts_with(
        "shared WAL coordination file is smaller than the coordination header"
    ))
}

/// Open the engine for the canonical `name`, waiting out a concurrent first
/// open of the same file (see [`is_coordination_file_uninitialized`]). Any
/// other error, or a companion that stays short for the whole budget,
/// surfaces as is; `path` names the file in the error as the caller wrote it.
pub(crate) async fn open(name: &str, read_only: bool, path: &Path) -> Result<turso::Database> {
    let started = Instant::now();
    loop {
        let opened = turso::Builder::new_local(name)
            .experimental_multiprocess_wal(name != ":memory:")
            .read_only(read_only)
            .build()
            .await;
        let error = match opened {
            Ok(database) => return Ok(database),
            Err(error) => error,
        };
        if is_coordination_file_uninitialized(&error)
            && started.elapsed() < COORDINATION_INIT_BUDGET
        {
            tracing::debug!(
                path = %path.display(),
                "WAL coordination file still being initialized by another process; retrying the open"
            );
            tokio::time::sleep(COORDINATION_INIT_PAUSE).await;
            continue;
        }
        return Err(anyhow::Error::new(error))
            .with_context(|| format!("open Turso database {}", path.display()));
    }
}

/// Lease `key` for a writer, waiting out whoever holds it incompatibly.
///
/// The file decides which lease the open needs. With no coordination
/// companion yet, this open may be the one that creates and initialises the
/// file, so it asks for the exclusive lease; once the companion exists, the
/// shared lease of an ordinary writer is enough. A conflict is retried and
/// the file is re-examined every round, because each conflict resolves on its
/// own: the process doing the first open downgrades to shared as soon as its
/// companion exists (and a later exclusive request can never succeed beside
/// it), and offline maintenance releases the exclusive lease when it is done.
/// Writers holding shared leases beside a missing companion -- a file an older
/// engine still has open -- are the one case that never resolves, and it
/// fails with the conflict after the budget rather than waiting forever.
async fn lease_for_writer(key: &Path) -> Result<MaintenanceLease> {
    let started = Instant::now();
    loop {
        let first_open = !coordination_path(key).exists();
        let attempt = if first_open {
            maintenance_lock::try_acquire_exclusive(key)
        } else {
            maintenance_lock::acquire_shared(key)
        };
        match attempt {
            Ok(lease) => return Ok(lease),
            Err(error)
                if maintenance_lock::is_writers_active(&error)
                    && started.elapsed() < LEASE_WAIT_BUDGET =>
            {
                tracing::debug!(
                    path = %key.display(),
                    first_open,
                    "maintenance lease contended; retrying the writer acquisition"
                );
                tokio::time::sleep(LEASE_WAIT_PAUSE).await;
            }
            Err(error) => return Err(error),
        }
    }
}

/// Lease `key` and open its engine; the returned lease is the one the caller
/// keeps for the engine's whole lifetime, so it must be dropped after the
/// engine (declare it last, or keep it in a tuple behind the engine). The
/// lease is taken before the engine opens the file: offline maintenance never
/// rewrites a file a writer is about to open, and a writer arriving beside
/// maintenance waits it out ([`lease_for_writer`]). A failed engine open
/// releases the lease with the error. `before_build` runs once the lease is
/// held and before the engine is built.
///
/// A first open runs under the exclusive lease and downgrades to shared right
/// after the engine is built: from then on the file is joinable by any writer,
/// and the lease says only that a writer is alive. Downgrading instead of
/// dropping and re-acquiring leaves no window for maintenance to slip in.
pub(crate) async fn open_writer<F, Fut>(
    key: &Path,
    path: &Path,
    before_build: F,
) -> Result<(turso::Database, MaintenanceLease)>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    let name = engine_name(key)?;
    let mut lease = lease_for_writer(key).await?;
    before_build().await;
    let database = open(name, false, path).await?;
    if lease.mode() == Mode::Exclusive {
        lease.downgrade()?;
    }
    // Tuple fields drop in order: the engine goes before the lease it ran under.
    Ok((database, lease))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_coordination_file_counts_as_another_process_initializing_it() {
        let short = turso::Error::Corrupt(
            "shared WAL coordination file is smaller than the coordination header: got 0, minimum 4096"
                .into(),
        );
        assert!(is_coordination_file_uninitialized(&short));
        let malformed = turso::Error::Corrupt("database disk image is malformed".into());
        assert!(!is_coordination_file_uninitialized(&malformed));
        let busy = turso::Error::Busy("database is locked".into());
        assert!(!is_coordination_file_uninitialized(&busy));
    }

    #[test]
    fn the_coordination_companion_sits_next_to_the_database() {
        assert_eq!(
            coordination_path(Path::new("/data/app.db")),
            PathBuf::from("/data/app.db-tshm")
        );
    }
}
