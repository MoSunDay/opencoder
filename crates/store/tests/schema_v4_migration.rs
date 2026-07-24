//! Schema v3 -> v4 migration: the `session_inputs.images_json` column is added
//! to carry multimodal image attachments, defaulting to `'[]'` so pre-existing
//! plain-text inputs read back with an empty `images` vec. Hand-writes a
//! faithful v3 database, reopens it (triggering `migrate(conn, 3)`), and
//! asserts the column + default + version bump.

use opencoder_store::{Delivery, LibsqlStore, SessionInput, SessionMeta, Store};

async fn raw_open(db_path: &std::path::Path) -> libsql::Connection {
    use libsql::Builder;
    let db = Builder::new_local(db_path).build().await.unwrap();
    db.connect().unwrap()
}

#[tokio::test]
async fn schema_v3_to_v4_adds_images_json_column() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("migrate-v4.db");

    // Phase 1: hand-write a v3 database. session_inputs lacks images_json;
    // sessions carries the full v3 shape (handoff_*/skill) and session_events
    // has sse_kind, so on reopen only the `if from < 4` branch runs.
    {
        let conn = raw_open(&db_path).await;
        conn.execute("CREATE TABLE schema_version (version INTEGER NOT NULL)", ())
            .await
            .unwrap();
        conn.execute(
            "CREATE TABLE sessions (               id TEXT PRIMARY KEY, title TEXT, agent TEXT, model TEXT, workdir_hash TEXT,               created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, summary TEXT, summary_seq INTEGER,               handoff_seq INTEGER, handoff_plan TEXT, skill TEXT)",
            (),
        )
        .await
        .unwrap();
        conn.execute(
            "CREATE TABLE session_inputs (               seq INTEGER PRIMARY KEY AUTOINCREMENT,               id TEXT NOT NULL,               session_id TEXT NOT NULL,               delivery TEXT NOT NULL,               prompt TEXT NOT NULL,               admitted_seq INTEGER NOT NULL,               promoted_seq INTEGER)",
            (),
        )
        .await
        .unwrap();
        conn.execute("INSERT INTO schema_version (version) VALUES (3)", ())
            .await
            .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES ('s1', 1, 1)",
            (),
        )
        .await
        .unwrap();
        // A pre-existing v3 input row: no images_json value.
        conn.execute(
            "INSERT INTO session_inputs (id, session_id, delivery, prompt, admitted_seq)             VALUES ('in-1', 's1', 'steer', 'old text', 1)",
            (),
        )
        .await
        .unwrap();
    }

    // Phase 2: reopen -> bootstrap -> migrate(conn, 3) runs only `if from < 4`.
    let store = LibsqlStore::open(&db_path).await.unwrap();

    // The pre-existing input reads back with empty images (column defaulted to
    // '[]'), proving backward compatibility and the DEFAULT clause.
    let pending = store.pending_inputs("s1", Delivery::Steer).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].prompt, "old text");
    assert!(
        pending[0].images.is_empty(),
        "v3 input must migrate to empty images, got {:?}",
        pending[0].images
    );

    // A newly admitted input WITH images round-trips through the migrated column.
    store
        .admit_input(&SessionInput {
            seq: None,
            id: "in-2".into(),
            session_id: "s1".into(),
            delivery: Delivery::Queue,
            prompt: "with image".into(),
            images: vec!["data:image/png;base64,YQ==".into()],
            admitted_seq: 2,
            promoted_seq: None,
        })
        .await
        .unwrap();
    let claimed = store.claim_next_queue("s1").await.unwrap();
    let (_, claimed_input) = claimed.expect("queued input claimed");
    assert_eq!(claimed_input.images, vec!["data:image/png;base64,YQ=="]);

    // Schema version bumped to 4.
    {
        let conn = store.conn().await.unwrap();
        let stmt = conn
            .prepare("SELECT version FROM schema_version LIMIT 1")
            .await
            .unwrap();
        let mut rows = stmt.query(()).await.unwrap();
        let r = rows.next().await.unwrap().unwrap();
        let v: i64 = r.get(0).unwrap();
        assert_eq!(v, 4, "schema version must be 4 after v3->v4 migration");
    }

    // Idempotent: reopening again does not error and the version stays at 4.
    drop(store);
    let store2 = LibsqlStore::open(&db_path).await.unwrap();
    let _ = store2
        .get_session("s1")
        .await
        .unwrap()
        .unwrap_or(SessionMeta::default());
    let conn = store2.conn().await.unwrap();
    let stmt = conn
        .prepare("SELECT version FROM schema_version LIMIT 1")
        .await
        .unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    let r = rows.next().await.unwrap().unwrap();
    let v: i64 = r.get(0).unwrap();
    assert_eq!(v, 4, "schema version stays 4 after idempotent re-open");
}
