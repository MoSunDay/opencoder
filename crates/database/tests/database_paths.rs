//! Every file-backed artifact is derived from the file a path resolves to:
//! opening through a symlink must place the WAL, the maintenance lease and
//! the engine marker next to the target, never next to the link, and
//! inspection through the link must see the live database.
#![cfg(unix)]
use opencoder_database::checkpoint::{self, wal_path};
use opencoder_database::maintenance_lock::{self, is_writers_active};
use opencoder_database::{maintenance, Database};
use serde_json::json;
use std::path::{Path, PathBuf};

const GATE: u64 = 8 * 1024;

/// Sorted entry names of `directory`.
fn entries(directory: &Path) -> Vec<String> {
    let mut names = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    names
}

/// One multi-row write transaction, sized so the WAL companion is written.
async fn fill(pool: &Database, rows: usize) {
    pool.write(move |connection| async move {
        connection
            .execute(
                "CREATE TABLE IF NOT EXISTS logs(id INTEGER PRIMARY KEY, text TEXT)",
                (),
            )
            .await?;
        let mut statement = connection
            .prepare("INSERT OR REPLACE INTO logs VALUES(?1, ?2)")
            .await?;
        for id in 0..rows {
            statement.execute((id as i64, "x".repeat(1000))).await?;
        }
        Ok(())
    })
    .await
    .unwrap();
}

/// The target directory of the database in the tests below.
fn real_dir(directory: &Path) {
    std::fs::create_dir(directory.join("real")).unwrap();
}

fn target(directory: &Path) -> PathBuf {
    directory.join("real").join("data.db")
}

/// Creating a database through a dangling symlink creates the target file and
/// keeps every companion -- WAL, maintenance lease, engine marker -- next to it.
#[tokio::test]
async fn a_dangling_symlink_is_resolved_before_any_artifact_is_created() {
    let directory = tempfile::tempdir().unwrap();
    real_dir(directory.path());
    let link = directory.path().join("link.db");
    std::os::unix::fs::symlink("real/data.db", &link).unwrap();

    let pool = Database::open(&link).await.unwrap();
    fill(&pool, 400).await;
    let target = target(directory.path());
    let key = link.canonicalize().unwrap();
    assert_eq!(key, target, "the link must resolve to the target");
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink(),
        "the link itself must not be replaced"
    );
    assert_eq!(
        entries(directory.path()),
        vec!["link.db", "real"],
        "no companion may appear next to the link"
    );
    assert!(
        checkpoint::wal_size(&target) >= GATE,
        "the WAL must grow next to the target, not next to the link"
    );
    for artifact in [
        maintenance_lock::lock_path(&target),
        PathBuf::from(format!("{}.engine.json", target.display())),
        wal_path(&target),
    ] {
        assert!(artifact.is_file(), "missing {}", artifact.display());
    }
    assert!(
        is_writers_active(&maintenance_lock::try_acquire_exclusive(&target).unwrap_err()),
        "the pool's shared lease must be recorded for the resolved file"
    );

    // The gated pass finds the pool's WAL through the link and merges it.
    let outcome = checkpoint::run_once(&link, GATE).await.unwrap();
    let checkpoint::Outcome::Merged { before_bytes, .. } = outcome else {
        panic!("the WAL reached through the link must be merged, got {outcome:?}");
    };
    assert!(
        before_bytes >= GATE,
        "the gate must see the real WAL: {before_bytes}"
    );

    // TRUNCATE through the link: refused while the pool is alive, then it
    // truncates the WAL of the file the link names.
    let refused = maintenance::checkpoint(link.clone()).await.unwrap_err();
    assert!(
        refused.to_string().contains("writers still active"),
        "the TRUNCATE must meet the pool's lease: {refused}"
    );
    drop(pool);
    maintenance::checkpoint(link.clone()).await.unwrap();
    assert_eq!(
        checkpoint::wal_size(&target),
        0,
        "TRUNCATE must leave the target's WAL empty"
    );
    assert!(
        !wal_path(&link).exists(),
        "TRUNCATE through the link must not create a WAL next to the link"
    );
    assert_eq!(
        entries(directory.path()),
        vec!["link.db", "real"],
        "still nothing next to the link"
    );
}

/// Opening a symlink to an existing database keeps using that database's own
/// WAL, and inspection through the link sees the committed rows.
#[tokio::test]
async fn a_symlink_to_an_existing_database_keeps_its_wal_and_rows_visible() {
    let directory = tempfile::tempdir().unwrap();
    real_dir(directory.path());
    let target = target(directory.path());
    let first = Database::open(&target).await.unwrap();
    fill(&first, 1).await;
    drop(first);

    let link = directory.path().join("link.db");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let pool = Database::open(&link).await.unwrap();
    fill(&pool, 400).await;
    assert!(
        !wal_path(&link).exists(),
        "no WAL may appear next to the link"
    );
    assert!(
        checkpoint::wal_size(&target) >= GATE,
        "the existing database's WAL must carry the writes"
    );
    assert_eq!(entries(directory.path()), vec!["link.db", "real"]);

    let seen = maintenance::query(link.clone(), "SELECT count(*) FROM logs".into(), vec![])
        .await
        .unwrap();
    assert_eq!(seen["rows"], json!([[400]]));
    let reader = Database::open_reader(&link).await.unwrap();
    let count = reader
        .read(|connection| async move {
            let mut rows = connection.query("SELECT count(*) FROM logs", ()).await?;
            Ok(rows.next().await.unwrap().unwrap().get::<i64>(0)?)
        })
        .await
        .unwrap();
    assert_eq!(count, 400);
    drop(reader);
    drop(pool);
}
