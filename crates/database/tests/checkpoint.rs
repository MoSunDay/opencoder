//! Periodic passive WAL checkpoint behavior: size gating, merging, and
//! non-blocking behavior under write-lock contention.
use opencoder_database::checkpoint::{self, Outcome};
use opencoder_database::Database;
use std::{path::Path, time::Duration};
use turso::Connection;

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

async fn populate(path: &Path, rows: usize) {
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
