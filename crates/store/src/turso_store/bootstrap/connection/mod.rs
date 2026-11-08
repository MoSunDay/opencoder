//! Bootstrap runs on the same isolated executor as later writes.
use super::{schema, tx};
use anyhow::Result;
use opencoder_database::Database;
use std::path::Path;

pub(super) async fn open_file(path: &Path) -> Result<Database> {
    open_with_reporting(path, true).await
}

pub(super) async fn open_catalog(path: &Path) -> Result<Database> {
    open_with_reporting(path, false).await
}

async fn open_with_reporting(path: &Path, runtime_reporting: bool) -> Result<Database> {
    // Capture pre-open existence: `Database::open` may itself create the
    // file, and the fresh-file checkpoint gate must not depend on that.
    let existed = path.exists();
    let started = std::time::Instant::now();
    let db = Database::open(path).await?;
    // Warm opens skip the write-locked bootstrap transaction entirely: the
    // probe runs on a read lane, so short-lived commands sharing this file
    // never contend for the cross-process write lock just to no-op the DDL.
    let skipped_bootstrap = db
        .read(move |conn| async move { schema::bootstrap_is_noop(&conn, runtime_reporting).await })
        .await
        .unwrap_or(false);
    if !skipped_bootstrap {
        db.write(move |conn| async move {
            schema::apply_connection_pragmas(&conn).await?;
            if runtime_reporting {
                schema::bootstrap(&conn).await?;
            } else {
                schema::bootstrap_catalog(&conn).await?;
            }
            if existed {
                // Best-effort: a concurrent writer or checkpointer holding
                // the lock is normal, and `wal_autocheckpoint` still bounds
                // the WAL, so a busy checkpoint must not fail the open.
                if let Err(error) = schema::checkpoint_wal(&conn).await {
                    if !tx::is_busy_conflict(&error) {
                        return Err(error);
                    }
                    tracing::debug!(error = %error, "wal checkpoint skipped (lock contention)");
                }
            }
            Ok(())
        })
        .await?;
    }
    tracing::info!(backend = "turso", path = %path.display(),
        total_ms = started.elapsed().as_millis() as u64,
        skipped_bootstrap, "store opened");
    Ok(db)
}
