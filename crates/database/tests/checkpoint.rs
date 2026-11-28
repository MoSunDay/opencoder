//! Periodic passive WAL checkpoint behavior: size gating, merging, and
//! non-blocking behavior under write-lock contention.
use opencoder_database::checkpoint::{self, Outcome};
use opencoder_database::Database;
use std::{path::Path, sync::Arc, time::Duration};
use turso::Connection;

mod support;
#[cfg(unix)]
use support::{entries, non_utf8_database};

async fn connect(path: &str, busy: Duration) -> Connection {
    let database = turso::Builder::new_local(path)
        .experimental_multiprocess_wal(true)
        .build()
        .await
        .unwrap();
    let conn = database.connect().unwrap();
    conn.busy_timeout(busy).unwrap();
    conn
}

/// Create `path` through a writer pool, fill it, and keep the pool alive so
/// its resident checkpoint handle stays the live one for the path.
async fn populated(path: &Path, rows: usize) -> Database {
    let database = Database::open(path).await.unwrap();
    database
        .write(|connection| async move {
            connection
                .execute("CREATE TABLE logs(id INTEGER PRIMARY KEY, text TEXT)", ())
                .await?;
            Ok(())
        })
        .await
        .unwrap();
    for batch in 0..rows.div_ceil(50) {
        let database = database.clone();
        database
            .write(move |connection| async move {
                for row in 0..50 {
                    let id = batch * 50 + row;
                    connection
                        .execute(
                            "INSERT INTO logs VALUES(?1, ?2)",
                            (id as i64, format!("payload-{id:05}-{}", "x".repeat(96))),
                        )
                        .await?;
                }
                Ok(())
            })
            .await
            .unwrap();
    }
    database
}

async fn populate(path: &Path, rows: usize) {
    populated(path, rows).await;
}

#[tokio::test]
async fn run_once_skips_below_threshold() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("small.db");
    populate(&path, 10).await;
    let outcome = checkpoint::run_once(&path, 64 * 1024 * 1024).await.unwrap();
    assert_eq!(
        outcome,
        Outcome::Skipped {
            wal_bytes: checkpoint::wal_size(&path)
        },
        "a WAL below the size gate must not be checkpointed"
    );
}

#[tokio::test]
async fn run_once_merges_wal_above_threshold() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("grown.db");
    populate(&path, 300).await;
    let outcome = checkpoint::run_once(&path, 8 * 1024).await.unwrap();
    let Outcome::Merged {
        before_bytes,
        after_bytes,
        checkpointed_pages,
    } = outcome
    else {
        panic!("WAL above the gate must be checkpointed, got {outcome:?}");
    };
    assert!(
        before_bytes >= 8 * 1024,
        "the WAL must exceed the gate before checkpointing: {before_bytes}"
    );
    assert!(checkpointed_pages > 0, "pages must be merged back");
    // Observed with turso 0.8.3: a PASSIVE checkpoint backfills every frame
    // (here 1_343_152 B WAL, 326 pages merged) but keeps the WAL file at its
    // high-water mark — writers reuse the file from the start instead of
    // growing it. File truncation stays the CLI-only TRUNCATE operation, so
    // the merge is observed through checkpointed pages and a WAL that does
    // not grow from the checkpoint itself.
    assert!(
        after_bytes <= before_bytes,
        "checkpoint must not grow the WAL: before={before_bytes} after={after_bytes}"
    );
}

#[tokio::test]
async fn run_once_reports_busy_without_blocking() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("held.db");
    populate(&path, 50).await;
    let holder = connect(path.to_str().unwrap(), Duration::from_secs(30)).await;
    holder.execute("BEGIN IMMEDIATE", ()).await.unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(5), async {
        checkpoint::run_once(&path, 1).await
    })
    .await
    .expect("a gated pass must never block on a held write lock")
    .unwrap();
    eprintln!("observed outcome under write lock: {outcome:?}");
    assert!(
        matches!(outcome, Outcome::Busy | Outcome::Merged { .. }),
        "contention must report busy or merge, not fail: {outcome:?}"
    );
    holder.execute("COMMIT", ()).await.unwrap();
}

#[test]
fn wal_path_matches_turso_layout() {
    assert_eq!(
        checkpoint::wal_path(Path::new("/tmp/x.db")),
        Path::new("/tmp/x.db-wal")
    );
}

#[tokio::test]
async fn periodic_checkpoint_reuses_the_pool_handle() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("reuse.db");
    // The pool stays alive: its handle is the live resident handle for the path.
    let pool = populated(&path, 1).await;
    let before = checkpoint::standalone_opens(&path);
    let a = checkpoint::handle_for(&path).await.unwrap();
    let b = checkpoint::handle_for(&path).await.unwrap();
    assert!(
        Arc::ptr_eq(&a, &b),
        "the same path must return the same resident handle"
    );
    assert_eq!(
        checkpoint::standalone_opens(&path),
        before,
        "a pool-registered handle must be reused, not reopened"
    );
    // The handle carries the pool's very engine instance, keyed by the
    // canonical path, rather than a reopened one.
    assert_eq!(a.path(), path.canonicalize().unwrap());
    // A gated pass must use the resident handle too.
    checkpoint::run_once(&path, 0).await.unwrap();
    assert_eq!(
        checkpoint::standalone_opens(&path),
        before,
        "a gated pass must not reopen the database"
    );
    let c = checkpoint::handle_for(&path).await.unwrap();
    assert!(
        Arc::ptr_eq(&a, &c),
        "the resident handle must survive a gated pass"
    );
    assert!(
        a.owner_alive() && pool.intent() == opencoder_database::Intent::Writer,
        "the live pool is what keeps the handle eligible"
    );
    drop(pool);
}

/// The resident handle owns ONE dedicated connection: pool establishment
/// primes it, and no successful pass calls `connect()` again (only a failed
/// pass discards it). Turso 0.8.3 runs its shared-WAL authority repair on
/// every `connect()`, which is the transient zero-page trigger, so
/// "connections opened" must stay at exactly one.
#[tokio::test]
async fn periodic_checkpoint_keeps_one_connection_per_handle() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("resident-connection.db");
    // Database::open primes the checkpoint connection; the pool stays alive.
    let pool = populated(&path, 1).await;
    let handle = checkpoint::handle_for(&path).await.unwrap();
    assert_eq!(
        handle.connection_opens(),
        1,
        "pool establishment must open the one checkpoint connection"
    );
    for _ in 0..2 {
        checkpoint::run_once(&path, 0).await.unwrap();
    }
    assert_eq!(
        handle.connection_opens(),
        1,
        "every pass must reuse the dedicated connection, never connect again"
    );
    let again = checkpoint::handle_for(&path).await.unwrap();
    assert!(
        Arc::ptr_eq(&handle, &again),
        "the handle (and its connection) must survive the passes"
    );
    drop(pool);
    assert!(
        !handle.owner_alive(),
        "dropping the pool must leave the retained handle without an owner"
    );
}

/// The CLI TRUNCATE checkpoint refuses a database path the engine cannot
/// name before it takes the lease: the refusal names the cause, is not a
/// writer conflict, and leaves no lease file or any other entry behind.
#[cfg(unix)]
#[tokio::test]
async fn truncate_checkpoint_refuses_a_non_utf8_path_before_any_lease_file() {
    use opencoder_database::maintenance_lock::{self, lock_path};
    let Some((directory, database)) = non_utf8_database() else {
        return;
    };
    let key = database.canonicalize().unwrap();
    assert!(
        key.to_str().is_none(),
        "fixture: the resolved path must stay non-UTF-8"
    );
    let refused = opencoder_database::maintenance::checkpoint(database.clone())
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("database path must be UTF-8"),
        "the refusal must name the cause: {refused:#}"
    );
    assert!(
        !maintenance_lock::is_writers_active(&refused),
        "an unnameable path is not a live writer: {refused:#}"
    );
    assert!(
        !lock_path(&key).exists(),
        "no lease file may be left behind"
    );
    assert_eq!(
        entries(directory.path()),
        vec![database.file_name().unwrap().to_os_string()],
        "nothing new may appear beside the database"
    );
}

/// The standalone checkpoint open refuses the same path before it takes its
/// own lease or builds an engine.
#[cfg(unix)]
#[tokio::test]
async fn handle_for_refuses_a_non_utf8_path_before_any_lease_file() {
    use opencoder_database::maintenance_lock::lock_path;
    let Some((directory, database)) = non_utf8_database() else {
        return;
    };
    let key = database.canonicalize().unwrap();
    assert!(
        key.to_str().is_none(),
        "fixture: the resolved path must stay non-UTF-8"
    );
    let refused = checkpoint::handle_for(&database)
        .await
        .err()
        .expect("non-UTF-8 path must be refused");
    assert!(
        refused.to_string().contains("database path must be UTF-8"),
        "the refusal must name the cause: {refused:#}"
    );
    assert!(
        !lock_path(&key).exists(),
        "no lease file may be left behind"
    );
    assert_eq!(
        checkpoint::standalone_opens(&database),
        0,
        "the refusal comes before the engine is built"
    );
    assert_eq!(
        entries(directory.path()),
        vec![database.file_name().unwrap().to_os_string()],
        "nothing new may appear beside the database"
    );
}
