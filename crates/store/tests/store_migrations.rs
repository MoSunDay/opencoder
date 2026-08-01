//! Schema-migration tests for the libsql-backed Store.
//!
//! Each test hand-writes an OLD on-disk schema (v1 / v2 / stale-version) with
//! `libsql::Builder`, then reopens through `LibsqlStore::open` to trigger
//! bootstrap -> migrate(), and asserts the resulting schema/version/behavior:
//! - schema_migration_versioning: bootstrap records schema version 6
//! - schema_migration_v1_to_v2_adds_sse_kind: v1 events gain sse_kind (NULL)
//! - schema_migration_v2_to_v3_adds_handoff_and_skill: v2 sessions gain the
//!   v3 handoff_seq/handoff_plan/skill columns
//! - schema_migration_is_idempotent_when_column_already_exists: stale
//!   schema_version with already-full tables must not error
//!
//! The hand-written old-schema DDL is preserved verbatim.

use opencoder_store::{LibsqlStore, SessionPatch, Store};
use tempfile::TempDir;

async fn fresh() -> (TempDir, LibsqlStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();
    (dir, store)
}

#[tokio::test]
async fn schema_migration_versioning() {
    let (_dir, store) = fresh().await;
    let conn = store.conn().await.unwrap();
    let stmt = conn
        .prepare("SELECT version FROM schema_version LIMIT 1")
        .await
        .unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    let r = rows.next().await.unwrap().expect("version row exists");
    let v: i64 = r.get(0).unwrap();
    assert_eq!(v, 6, "schema_version must be 6 after bootstrap");
}

#[tokio::test]
async fn schema_migration_v1_to_v2_adds_sse_kind() {
    use libsql::Builder;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("migrate.db");

    // Phase 1: manually create a v1-style database (no sse_kind column).
    {
        let db = Builder::new_local(&db_path).build().await.unwrap();
        let conn = db.connect().unwrap();
        conn.execute("CREATE TABLE schema_version (version INTEGER NOT NULL)", ())
            .await
            .unwrap();
        conn.execute(
            "CREATE TABLE sessions (\
               id TEXT PRIMARY KEY, title TEXT, agent TEXT, model TEXT, workdir_hash TEXT,\
               created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, summary TEXT, summary_seq INTEGER)",
            (),
        )
        .await
        .unwrap();
        conn.execute(
            "CREATE TABLE session_events (\
               seq INTEGER PRIMARY KEY AUTOINCREMENT,\
               session_id TEXT NOT NULL,\
               type TEXT NOT NULL, payload_json TEXT NOT NULL,\
               ts INTEGER NOT NULL)",
            (),
        )
        .await
        .unwrap();
        conn.execute("INSERT INTO schema_version (version) VALUES (1)", ())
            .await
            .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES ('s1', 1, 1)",
            (),
        )
        .await
        .unwrap();
        conn.execute(
            "INSERT INTO session_events (session_id, type, payload_json, ts) \
             VALUES ('s1', 'text_delta', '{}', 100)",
            (),
        )
        .await
        .unwrap();
    }

    // Phase 2: reopen — triggers bootstrap → migrate from v1 to v2.
    let store = LibsqlStore::open(&db_path).await.unwrap();

    // Old event record: sse_kind is None (column added by migration, was NULL).
    let events = store.events_after("s1", 0).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].sse_kind, None,
        "old v1 events should have sse_kind=None"
    );

    // Schema version bumped to 2.
    {
        let conn = store.conn().await.unwrap();
        let stmt = conn
            .prepare("SELECT version FROM schema_version LIMIT 1")
            .await
            .unwrap();
        let mut rows = stmt.query(()).await.unwrap();
        let r = rows.next().await.unwrap().unwrap();
        let v: i64 = r.get(0).unwrap();
        assert_eq!(v, 6, "schema version must be 6 after migration");
    }

    // New events can be stored with sse_kind and read back.
    use opencoder_store::EventKind;
    store
        .append_event(&opencoder_store::SessionEventRecord {
            session_id: "s1".into(),
            kind: EventKind::Step,
            payload: serde_json::json!({"status": "ok"}),
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
    assert_eq!(v, 6, "schema version stays 6 after idempotent re-open");
}

#[tokio::test]
async fn schema_migration_v2_to_v3_adds_handoff_and_skill() {
    use libsql::Builder;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("migrate-v3.db");

    // Phase 1: hand-write a faithful v2 database. The sessions table omits the
    // v3 columns (handoff_seq / handoff_plan / skill), and session_events ALREADY
    // has sse_kind — so on reopen migrate(conn, 2) skips the `if from < 2` block
    // and only runs the `if from < 3` block, isolating the v3 migration branch.
    {
        let db = Builder::new_local(&db_path).build().await.unwrap();
        let conn = db.connect().unwrap();
        conn.execute("CREATE TABLE schema_version (version INTEGER NOT NULL)", ())
            .await
            .unwrap();
        // sessions at v2: no handoff_seq / handoff_plan / skill columns yet.
        conn.execute(
            "CREATE TABLE sessions (\
               id TEXT PRIMARY KEY, title TEXT, agent TEXT, model TEXT, workdir_hash TEXT,\
               created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, summary TEXT, summary_seq INTEGER)",
            (),
        )
        .await
        .unwrap();
        // session_events at v2: already carries sse_kind.
        conn.execute(
            "CREATE TABLE session_events (\
               seq INTEGER PRIMARY KEY AUTOINCREMENT,\
               session_id TEXT NOT NULL,\
               type TEXT NOT NULL, payload_json TEXT NOT NULL,\
               sse_kind TEXT,\
               ts INTEGER NOT NULL)",
            (),
        )
        .await
        .unwrap();
        conn.execute("INSERT INTO schema_version (version) VALUES (2)", ())
            .await
            .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES ('s2', 1, 1)",
            (),
        )
        .await
        .unwrap();
    }

    // Phase 2: reopen — triggers bootstrap → migrate(conn, 2), which runs only
    // the `if from < 3` branch (adds handoff_seq / handoff_plan / skill).
    let store = LibsqlStore::open(&db_path).await.unwrap();

    // (1) The pre-existing row survives; the new columns are nullable, so the
    //     three v3 fields read back as None without data loss.
    let m0 = store.get_session("s2").await.unwrap().unwrap();
    assert_eq!(m0.id, "s2");
    assert!(m0.handoff_seq.is_none(), "v2 row: handoff_seq must be None");
    assert!(
        m0.handoff_plan.is_none(),
        "v2 row: handoff_plan must be None"
    );
    assert!(m0.skill.is_none(), "v2 row: skill must be None");

    // (2) The migrated columns round-trip through SessionPatch (write + read).
    store
        .update_session(
            "s2",
            &SessionPatch {
                handoff_seq: Some(42),
                handoff_plan: Some("## Plan\n1. a\n2. b".into()),
                skill: Some("review".into()),
                updated_at: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let m1 = store.get_session("s2").await.unwrap().unwrap();
    assert_eq!(m1.handoff_seq, Some(42));
    assert_eq!(m1.handoff_plan.as_deref(), Some("## Plan\n1. a\n2. b"));
    assert_eq!(m1.skill.as_deref(), Some("review"));

    // (3) Schema version bumped to 3.
    {
        let conn = store.conn().await.unwrap();
        let stmt = conn
            .prepare("SELECT version FROM schema_version LIMIT 1")
            .await
            .unwrap();
        let mut rows = stmt.query(()).await.unwrap();
        let r = rows.next().await.unwrap().unwrap();
        let v: i64 = r.get(0).unwrap();
        assert_eq!(v, 6, "schema version must be 6 after v2→v3 migration");
    }

    // (4) Idempotent: reopening again does not re-run migration or error, and
    //     the version stays at 4.
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
    assert_eq!(v, 6, "schema version stays 6 after idempotent re-open");
}

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
        assert_eq!(v, 6, "schema version must be 6 after migration");
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
    assert_eq!(v, 6, "schema version stays 6 after idempotent re-open");
}
