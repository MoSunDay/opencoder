//! Stale-shape and session summary schema migrations.

use opencoder_store::{LibsqlStore, SessionPatch, Store};

#[tokio::test]
async fn schema_migration_is_idempotent_when_column_already_exists() {
    use libsql::Builder;
    use opencoder_store::{EventKind, SessionEventRecord};

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("idempotent-migrate.db");

    // Reproduce the exact failure mode: the on-disk tables already carry the
    // *full latest* shape (CREATE TABLE statements embed the full schema, so
    // they include e.g. sse_kind on session_events and handoff_seq/handoff_plan/
    // skill on sessions), but schema_version is stale at 1. A bare ADD COLUMN
    // in migrate() would fail with `duplicate column name: sse_kind`.
    {
        let db = Builder::new_local(&db_path).build().await.unwrap();
        let conn = db.connect().unwrap();
        conn.execute("CREATE TABLE schema_version (version INTEGER NOT NULL)", ())
            .await
            .unwrap();
        // sessions with the full current shape, including the v3 handoff/skill
        // columns — identical to the CREATE_SESSIONS the store ships.
        conn.execute(
            "CREATE TABLE sessions (\
               id TEXT PRIMARY KEY, title TEXT, agent TEXT, model TEXT, workdir_hash TEXT,\
               created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, summary TEXT,\
               summary_seq INTEGER, handoff_seq INTEGER, handoff_plan TEXT, skill TEXT)",
            (),
        )
        .await
        .unwrap();
        // session_events with the full current shape, including the v2 sse_kind
        // column — identical to CREATE_EVENTS the store ships.
        conn.execute(
            "CREATE TABLE session_events (\
               seq INTEGER PRIMARY KEY AUTOINCREMENT,\
               session_id TEXT NOT NULL,\
               type TEXT NOT NULL, payload_json TEXT NOT NULL,\
               sse_kind TEXT, ts INTEGER NOT NULL)",
            (),
        )
        .await
        .unwrap();
        // Stale version: schema_version = 1, but tables are already at v3 shape.
        conn.execute("INSERT INTO schema_version (version) VALUES (1)", ())
            .await
            .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES ('s1', 1, 1)",
            (),
        )
        .await
        .unwrap();
        // Pre-existing event carrying a real sse_kind value that must survive.
        conn.execute(
            "INSERT INTO session_events (session_id, type, payload_json, sse_kind, ts) \
             VALUES ('s1', 'step', '{\"status\":\"ok\"}', 'status', 100)",
            (),
        )
        .await
        .unwrap();
    }

    // Reopen — triggers bootstrap → migrate(1). Before the fix this errored:
    //   `migrate v2: add sse_kind column` / `duplicate column name: sse_kind`.
    let store = LibsqlStore::open(&db_path).await.unwrap();

    // The pre-existing sse_kind data is intact and reads back through the store.
    let events = store.events_after("s1", 0).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].sse_kind.as_deref(),
        Some("status"),
        "pre-existing sse_kind data must survive migration"
    );

    // schema_version bumped all the way to 3.
    {
        let conn = store.conn().await.unwrap();
        let stmt = conn
            .prepare("SELECT version FROM schema_version LIMIT 1")
            .await
            .unwrap();
        let mut rows = stmt.query(()).await.unwrap();
        let r = rows.next().await.unwrap().unwrap();
        let v: i64 = r.get(0).unwrap();
        assert_eq!(v, 23, "schema version must be latest (23) after migration");
    }

    // A freshly appended event still round-trips its sse_kind.
    store
        .append_event(&SessionEventRecord {
            session_id: "s1".into(),
            kind: EventKind::Step,
            payload: serde_json::json!({"status": "more"}),
            ts: 200,
            seq: None,
            sse_kind: Some("status".into()),
        })
        .await
        .unwrap();
    let events2 = store.events_after("s1", 0).await.unwrap();
    assert_eq!(events2.len(), 2);
    assert_eq!(events2[1].sse_kind.as_deref(), Some("status"));

    // Idempotent: reopening again does not re-run migration or error.
    drop(store);
    let store2 = LibsqlStore::open(&db_path).await.unwrap();
    let conn = store2.conn().await.unwrap();
    let stmt = conn
        .prepare("SELECT version FROM schema_version LIMIT 1")
        .await
        .unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    let r = rows.next().await.unwrap().unwrap();
    let v: i64 = r.get(0).unwrap();
    assert_eq!(v, 23, "schema version stays 23 after idempotent re-open");
}

/// v6 -> v7: reopening a faithful v6 database (sessions WITHOUT
/// `summary_images_json`) must add the column so compaction images can be
/// persisted, and pre-existing rows read back as an empty vec (NULL default).
#[tokio::test]
async fn schema_migration_v6_to_v7_adds_summary_images() {
    use libsql::Builder;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("migrate-v7.db");

    // Phase 1: hand-write a v6 sessions table (no summary_images_json column).
    {
        let db = Builder::new_local(&db_path).build().await.unwrap();
        let conn = db.connect().unwrap();
        conn.execute("CREATE TABLE schema_version (version INTEGER NOT NULL)", ())
            .await
            .unwrap();
        // sessions at v6: full column set EXCEPT summary_images_json.
        conn.execute(
            "CREATE TABLE sessions (\
               id TEXT PRIMARY KEY, title TEXT, agent TEXT, model TEXT, workdir_hash TEXT,\
               created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,\
               summary TEXT, summary_seq INTEGER, handoff_seq INTEGER, handoff_plan TEXT,\
               skill TEXT, task_type TEXT NOT NULL DEFAULT 'parent')",
            (),
        )
        .await
        .unwrap();
        conn.execute("INSERT INTO schema_version (version) VALUES (6)", ())
            .await
            .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES ('s6', 1, 1)",
            (),
        )
        .await
        .unwrap();
    }

    // Phase 2: reopen — migrate(conn, 6) runs the `if from < 7` block, adding
    // summary_images_json to sessions.
    let store = LibsqlStore::open(&db_path).await.unwrap();

    // (1) Pre-existing row survives; the new column is NULL -> empty vec.
    let m0 = store.get_session("s6").await.unwrap().unwrap();
    assert_eq!(m0.id, "s6");
    assert!(
        m0.summary_images.is_empty(),
        "v6 row: summary_images reads as empty"
    );

    // (2) The migrated column round-trips through SessionPatch.
    store
        .update_session(
            "s6",
            &SessionPatch {
                summary_images: Some(vec!["img-a.png".into(), "img-b.png".into()]),
                updated_at: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let m1 = store.get_session("s6").await.unwrap().unwrap();
    assert_eq!(
        m1.summary_images,
        vec!["img-a.png".to_string(), "img-b.png".to_string()],
        "migrated summary_images_json round-trips"
    );

    // (3) Schema version bumped to 9.
    {
        let conn = store.conn().await.unwrap();
        let stmt = conn
            .prepare("SELECT version FROM schema_version LIMIT 1")
            .await
            .unwrap();
        let mut rows = stmt.query(()).await.unwrap();
        let r = rows.next().await.unwrap().unwrap();
        let v: i64 = r.get(0).unwrap();
        assert_eq!(
            v, 23,
            "schema version must be latest (23) after v6->v7 migration"
        );
    }
}
