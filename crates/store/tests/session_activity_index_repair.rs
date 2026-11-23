//! Regression for `persist assistant response: touch session activity in tx:
//! IdxDelete: no matching index entry found for key [...] while seeking`.
//!
//! `idx_sessions_activity` is derived from `sessions.updated_at`. When the
//! index and the table row disagree (storage-level page inconsistency), every
//! `UPDATE sessions SET updated_at = ...` fails with `SQLITE_CORRUPT` because
//! the old index key computed from the row is missing. These tests pin the
//! two repairs: the v41 migration rebuilds the index as a plain column index
//! from table data, and the append path detects the corruption, rebuilds the
//! index and retries the transaction once.

use std::path::Path;

use opencoder_core::Message;
use opencoder_store::{SessionFilter, SessionMeta, Store, TursoStore, TASK_TYPE_PARENT};

fn meta(id: &str, created_at: i64, updated_at: i64) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        title: Some(id.into()),
        agent: Some("act".into()),
        model: None,
        workdir_hash: None,
        created_at,
        updated_at,
        summary: None,
        summary_seq: None,
        summary_images: vec![],
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: Some(TASK_TYPE_PARENT.into()),
        requirement: None,
        kind: None,
    }
}

fn msg(id: &str, created_at: i64) -> Message {
    let mut m = Message::user(format!("m-{id}"), format!("text-{id}"));
    m.created_at = created_at;
    m
}

async fn checkpoint_truncate(conn: &turso::Connection) {
    let mut rows = conn
        .query("PRAGMA wal_checkpoint(TRUNCATE)", ())
        .await
        .unwrap();
    let row = rows.next().await.unwrap().expect("checkpoint result row");
    assert_eq!(row.get::<i64>(0).unwrap(), 0, "checkpoint was busy");
}

async fn index_sql(store: &TursoStore) -> String {
    let conn = store.conn().await.unwrap();
    let mut rows = conn
        .query(
            "SELECT sql FROM sqlite_master WHERE name = 'idx_sessions_activity'",
            (),
        )
        .await
        .unwrap();
    rows.next()
        .await
        .unwrap()
        .expect("activity index exists")
        .get::<String>(0)
        .unwrap()
}

async fn list_ids(store: &TursoStore) -> Vec<String> {
    store
        .list_sessions(&SessionFilter::default())
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect()
}

/// Simulate the production corruption: roll the `sessions` table page back to
/// an older snapshot while `idx_sessions_activity` keeps the newer entry.
/// Restoring a single 4 KiB page from a snapshot of the same file keeps the
/// rest of the database (messages, schema) intact.
async fn restore_table_page(path: &Path, snapshot: &[u8], table_root: i64, page_size: i64) {
    let mut current = std::fs::read(path).unwrap();
    let start = (table_root - 1) as usize * page_size as usize;
    let end = start + page_size as usize;
    current[start..end].copy_from_slice(&snapshot[start..end]);
    std::fs::write(path, current).unwrap();
}

#[tokio::test]
async fn append_self_heals_a_desynced_activity_index() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("repair.db");

    // Phase 1: session row + index entry both at 100, then snapshot the file.
    let (snapshot, table_root, page_size) = {
        let store = TursoStore::open(&path).await.unwrap();
        store.create_session(&meta("s1", 100, 100)).await.unwrap();
        store.append_message("s1", &msg("m1", 100)).await.unwrap();
        let conn = store.conn().await.unwrap();
        checkpoint_truncate(&conn).await;
        let page_size = {
            let mut rows = conn.query("PRAGMA page_size", ()).await.unwrap();
            rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap()
        };
        let table_root = {
            let mut rows = conn
                .query(
                    "SELECT rootpage FROM sqlite_master WHERE name = 'sessions'",
                    (),
                )
                .await
                .unwrap();
            rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap()
        };
        let snapshot = std::fs::read(&path).unwrap();
        // Phase 2: advance the activity stamp (table + index) and checkpoint.
        store.append_message("s1", &msg("m2", 500)).await.unwrap();
        checkpoint_truncate(&conn).await;
        (snapshot, table_root, page_size)
    };

    // Phase 3: table page back to 100, index page still at 500.
    restore_table_page(&path, &snapshot, table_root, page_size).await;

    let store = TursoStore::open(&path).await.unwrap();
    {
        let conn = store.conn().await.unwrap();
        let table_stamp = {
            let mut rows = conn
                .query("SELECT updated_at FROM sessions WHERE id = 's1'", ())
                .await
                .unwrap();
            rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap()
        };
        let index_stamp = {
            let mut rows = conn
                .query(
                    "SELECT updated_at FROM sessions INDEXED BY idx_sessions_activity WHERE id = 's1'",
                    (),
                )
                .await
                .unwrap();
            rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap()
        };
        assert_eq!(table_stamp, 100, "table page rolled back");
        assert_eq!(index_stamp, 500, "index page kept the newer entry");
    }

    // The append whose touch UPDATE would fail with `IdxDelete: no matching
    // index entry found` heals the index and persists the message.
    store.append_message("s1", &msg("m3", 600)).await.unwrap();

    let messages = store.load_messages("s1").await.unwrap();
    assert_eq!(messages.len(), 3, "all appends persisted");
    assert!(messages.iter().any(|m| m.id == "m-m3"));

    let conn = store.conn().await.unwrap();
    let (table_stamp, index_stamp) = {
        let mut rows = conn
            .query(
                "SELECT s.updated_at,
                        (SELECT i.updated_at FROM sessions i INDEXED BY idx_sessions_activity
                          WHERE i.id = s.id)
                 FROM sessions s WHERE s.id = 's1'",
                (),
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        (row.get::<i64>(0).unwrap(), row.get::<i64>(1).unwrap())
    };
    assert_eq!(table_stamp, 600);
    assert_eq!(index_stamp, 600, "index rebuilt from table data");
}

#[tokio::test]
async fn migration_v40_rebuilds_expression_index_and_backfills_activity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("migrate-v41.db");
    {
        let store = TursoStore::open(&path).await.unwrap();
        let conn = store.conn().await.unwrap();
        // A pre-v41 row written directly: updated_at below created_at (the
        // shape imported backfills carry), bypassing the insert clamp.
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES ('s_backfill', 500, 100)",
            (),
        )
        .await
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES ('s_normal', 100, 900)",
            (),
        )
        .await
        .unwrap();
        // Recreate the pre-v41 shape: derived expression index + watermark 40.
        conn.execute_batch(
            "DROP INDEX idx_sessions_activity;
             CREATE INDEX idx_sessions_activity ON sessions(MAX(updated_at, created_at) DESC, id DESC);
             UPDATE schema_version SET version = 40;",
        )
        .await
        .unwrap();
    }

    let store = TursoStore::open(&path).await.unwrap();

    // The derived expression index was replaced by the plain column index.
    let sql = index_sql(&store).await;
    assert!(
        sql.contains("updated_at DESC, id DESC"),
        "plain activity index expected, got: {sql}"
    );
    assert!(!sql.contains("MAX"), "expression index must be gone: {sql}");

    // The activity invariant was backfilled.
    let conn = store.conn().await.unwrap();
    let backfill = {
        let mut rows = conn
            .query(
                "SELECT updated_at FROM sessions WHERE id = 's_backfill'",
                (),
            )
            .await
            .unwrap();
        rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap()
    };
    assert_eq!(backfill, 500, "updated_at clamped up to created_at");

    // Listing order and appends work on the rebuilt index.
    assert_eq!(list_ids(&store).await, ["s_normal", "s_backfill"]);
    store
        .append_message("s_backfill", &msg("m1", 700))
        .await
        .unwrap();
    assert_eq!(list_ids(&store).await, ["s_normal", "s_backfill"]);
    let mut rows = conn
        .query(
            "SELECT updated_at FROM sessions INDEXED BY idx_sessions_activity WHERE id = 's_backfill'",
            (),
        )
        .await
        .unwrap();
    assert_eq!(
        rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
        700
    );
}
