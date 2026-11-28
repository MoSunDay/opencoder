use opencoder_database::maintenance_lock;
use opencoder_database::{maintenance, Database};
use serde_json::json;

#[tokio::test]
async fn inspection_rejects_writes_and_backup_preserves_a_live_database() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.db");
    let output = dir.path().join("copy.db");
    let db = Database::open(&source).await.unwrap();
    db.write(|conn| async move {
        conn.execute_batch("CREATE TABLE items(id INTEGER PRIMARY KEY, value TEXT); INSERT INTO items VALUES(1,'before');").await?;
        Ok(())
    }).await.unwrap();
    let before = maintenance::query(
        source.clone(),
        "SELECT value FROM items WHERE id=?1".into(),
        vec![json!(1)],
    )
    .await
    .unwrap();
    assert_eq!(before["rows"], json!([["before"]]));
    assert!(
        maintenance::query(source.clone(), "DELETE FROM items".into(), vec![])
            .await
            .is_err()
    );
    maintenance::backup(source.clone(), output.clone())
        .await
        .unwrap();
    db.write(|conn| async move {
        conn.execute("UPDATE items SET value='after'", ()).await?;
        Ok(())
    })
    .await
    .unwrap();
    let saved = maintenance::query(output.clone(), "SELECT value FROM items".into(), vec![])
        .await
        .unwrap();
    assert_eq!(saved["rows"], json!([["before"]]));
    assert!(maintenance::backup(source, output).await.is_err());
}

/// One multi-row write transaction, sized so the WAL companion is written.
async fn fill(pool: &Database, rows: usize) {
    pool.write(move |connection| async move {
        connection
            .execute("CREATE TABLE logs(id INTEGER PRIMARY KEY, text TEXT)", ())
            .await?;
        let mut statement = connection
            .prepare("INSERT INTO logs VALUES(?1, ?2)")
            .await?;
        for id in 0..rows {
            statement.execute((id as i64, "x".repeat(1000))).await?;
        }
        Ok(())
    })
    .await
    .unwrap();
}

/// TRUNCATE needs the exclusive lease a live pool's shared lease refuses:
/// refused with "writers still active" while the pool is alive, then the WAL
/// is truncated once the pool is gone and its lease with it. The refusal is
/// typed both ways: the operator-facing text and the `WritersActive` cause in
/// the chain.
#[tokio::test]
async fn truncate_checkpoint_refuses_the_pool_and_then_empties_the_wal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("truncate.db");
    let pool = Database::open(&path).await.unwrap();
    fill(&pool, 400).await;
    assert!(
        opencoder_database::checkpoint::wal_size(&path) >= 8 * 1024,
        "the WAL must exist before the TRUNCATE"
    );
    let refused = maintenance::checkpoint(path.clone()).await.unwrap_err();
    assert!(
        refused.to_string().contains("writers still active"),
        "the TRUNCATE must meet the live pool's lease: {refused}"
    );
    assert!(
        maintenance_lock::is_writers_active(&refused),
        "the live writer is the named cause: {refused:#}"
    );
    assert!(
        refused
            .to_string()
            .contains(&path.canonicalize().unwrap().display().to_string()),
        "the refusal names the database: {refused:#}"
    );
    // Writers never exclude each other: a second open of the file is one
    // more writer, not a refusal, and the TRUNCATE stays refused for it.
    let second = Database::open(&path).await.unwrap();
    let still_refused = maintenance::checkpoint(path.clone())
        .await
        .expect_err("a second writer is one more live writer");
    assert!(
        maintenance_lock::is_writers_active(&still_refused),
        "{still_refused:#}"
    );
    drop(second);
    assert!(
        opencoder_database::checkpoint::wal_size(&path) >= 8 * 1024,
        "a refused TRUNCATE must leave the WAL untouched"
    );
    drop(pool);
    maintenance::checkpoint(path.clone()).await.unwrap();
    assert_eq!(
        opencoder_database::checkpoint::wal_size(&path),
        0,
        "TRUNCATE must empty the WAL"
    );
}

/// Only a real lease conflict means "writers still active". Any other failure
/// of the lease step keeps its own cause, so an operator is not told to stop
/// writers that were never running. The lease file is made a directory so the
/// lease cannot be taken for a reason unrelated to contention.
#[tokio::test]
async fn truncate_checkpoint_keeps_lease_failures_that_are_not_a_live_writer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("unlockable.db");
    let pool = Database::open(&path).await.unwrap();
    fill(&pool, 1).await;
    drop(pool);
    let key = path.canonicalize().unwrap();
    std::fs::remove_file(maintenance_lock::lock_path(&key)).unwrap();
    std::fs::create_dir(maintenance_lock::lock_path(&key)).unwrap();
    let error = maintenance::checkpoint(path.clone()).await.unwrap_err();
    assert!(
        !error.to_string().contains("writers still active"),
        "an I/O failure of the lease step is not a live writer: {error:#}"
    );
    assert!(
        !maintenance_lock::is_writers_active(&error),
        "no writer holds the database: {error}"
    );
    assert!(
        error
            .to_string()
            .contains(&maintenance_lock::lock_path(&key).display().to_string()),
        "the refusal names the lease file: {error:#}"
    );
}

/// A reader takes no lease, so it is never reported as a writer: a reader
/// holding a read snapshot makes the engine report the TRUNCATE as busy (the
/// generic busy text mentions writers, but no `WritersActive` cause is
/// attached), and the same call goes through once the snapshot ends.
#[tokio::test]
async fn a_reader_snapshot_makes_truncate_busy_but_never_counts_as_a_writer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reader.db");
    let pool = Database::open(&path).await.unwrap();
    fill(&pool, 400).await;
    drop(pool);
    assert!(
        opencoder_database::checkpoint::wal_size(&path) >= 8 * 1024,
        "the WAL must exist before the TRUNCATE"
    );

    let reader = Database::open_reader(&path).await.unwrap();
    let snapshot = reader.connect().await.unwrap();
    snapshot.execute("BEGIN", ()).await.unwrap();
    let mut rows = snapshot
        .query("SELECT count(*) FROM logs", ())
        .await
        .unwrap();
    assert!(rows.next().await.unwrap().is_some());

    let busy = maintenance::checkpoint(path.clone()).await.unwrap_err();
    let chain = format!("{busy:#}");
    assert!(
        chain.contains("checkpoint is busy"),
        "an open read snapshot makes the engine report busy: {chain}"
    );
    assert!(
        !maintenance_lock::is_writers_active(&busy),
        "a reader is not a writer: {chain}"
    );
    let key = path.canonicalize().unwrap();
    assert!(
        maintenance_lock::try_acquire_exclusive(&key).is_ok(),
        "a reader holds no lease: offline maintenance gets the lease beside it"
    );
    assert!(
        opencoder_database::checkpoint::wal_size(&path) >= 8 * 1024,
        "a busy TRUNCATE must leave the WAL untouched"
    );

    snapshot.execute("ROLLBACK", ()).await.unwrap();
    drop(snapshot);
    maintenance::checkpoint(path.clone()).await.unwrap();
    assert_eq!(opencoder_database::checkpoint::wal_size(&path), 0);
}
