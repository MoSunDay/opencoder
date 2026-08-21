//! Schema-migration tests for the libsql-backed Store.
//!
//! Each test hand-writes an OLD on-disk schema (v1 / v2 / stale-version) with
//! `libsql::Builder`, then reopens through `LibsqlStore::open` to trigger
//! bootstrap -> migrate(), and asserts the resulting schema/version/behavior:
//! - schema_migration_versioning: bootstrap records schema version 7
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
    assert_eq!(v, 11, "schema_version must be 11 after bootstrap");
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
        assert_eq!(v, 11, "schema version must be 11 after migration");
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
    assert_eq!(v, 11, "schema version stays 11 after idempotent re-open");
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
        assert_eq!(v, 11, "schema version must be 11 after v2→v3 migration");
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
    assert_eq!(v, 11, "schema version stays 11 after idempotent re-open");
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
        assert_eq!(v, 11, "schema version must be 11 after migration");
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
    assert_eq!(v, 11, "schema version stays 11 after idempotent re-open");
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
        assert_eq!(v, 11, "schema version must be 11 after v6->v7 migration");
    }
}

// ===========================================================================
// v7 -> v8 migration: the `sessions.requirement` column.
// ===========================================================================

/// Hand-write a faithful v7 sessions table (carries `summary_images_json` but
/// NOT `requirement`), then reopen through `LibsqlStore::open` so
/// `migrate(conn, 7)` runs the `if from < 8` block. Asserts the column is
/// added (NULL by default), round-trips through `SessionPatch`, and that the
/// schema version bumps to 8.
#[tokio::test]
async fn schema_migration_v7_to_v8_adds_requirement() {
    use libsql::Builder;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("migrate-v8.db");

    // Phase 1: hand-write a v7 sessions table. It has the full v7 column set
    // (including summary_images_json) but NO requirement column.
    {
        let db = Builder::new_local(&db_path).build().await.unwrap();
        let conn = db.connect().unwrap();
        conn.execute("CREATE TABLE schema_version (version INTEGER NOT NULL)", ())
            .await
            .unwrap();
        // sessions at v7: full column set EXCEPT requirement.
        conn.execute(
            "CREATE TABLE sessions (\
               id TEXT PRIMARY KEY, title TEXT, agent TEXT, model TEXT, workdir_hash TEXT,\
               created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,\
               summary TEXT, summary_seq INTEGER, summary_images_json TEXT,\
               handoff_seq INTEGER, handoff_plan TEXT, skill TEXT,\
               task_type TEXT NOT NULL DEFAULT 'parent')",
            (),
        )
        .await
        .unwrap();
        conn.execute("INSERT INTO schema_version (version) VALUES (7)", ())
            .await
            .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES ('s7', 1, 1)",
            (),
        )
        .await
        .unwrap();
    }

    // Phase 2: reopen — migrate(conn, 7) runs the `if from < 8` block, adding
    // the requirement column to sessions.
    let store = LibsqlStore::open(&db_path).await.unwrap();

    // (1) The requirement column now exists on sessions (and is NULL).
    {
        let conn = store.conn().await.unwrap();
        let stmt = conn.prepare("PRAGMA table_info(sessions)").await.unwrap();
        let mut rows = stmt.query(()).await.unwrap();
        let mut found = false;
        while let Some(row) = rows.next().await.unwrap() {
            let name: String = row.get(1).unwrap();
            if name == "requirement" {
                found = true;
                break;
            }
        }
        assert!(
            found,
            "sessions.requirement column must exist after v7->v8 migration"
        );
    }

    // (2) Pre-existing row survives; the new column reads back as NULL/None.
    let m0 = store.get_session("s7").await.unwrap().unwrap();
    assert_eq!(m0.id, "s7");
    assert_eq!(
        m0.requirement, None,
        "v7 row: requirement reads as NULL after migration"
    );

    // (3) The migrated column round-trips through SessionPatch.
    store
        .update_session(
            "s7",
            &SessionPatch {
                requirement: Some("test".into()),
                updated_at: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let m1 = store.get_session("s7").await.unwrap().unwrap();
    assert_eq!(
        m1.requirement.as_deref(),
        Some("test"),
        "migrated requirement round-trips"
    );

    // (4) Schema version bumped to 9.
    {
        let conn = store.conn().await.unwrap();
        let stmt = conn
            .prepare("SELECT version FROM schema_version LIMIT 1")
            .await
            .unwrap();
        let mut rows = stmt.query(()).await.unwrap();
        let r = rows.next().await.unwrap().unwrap();
        let v: i64 = r.get(0).unwrap();
        assert_eq!(v, 11, "schema version must be 11 after v7->v8 migration");
    }
}

// ===========================================================================
// v10 -> v11 migration: the `sessions.autopilot_mode` column.
// ===========================================================================

/// Hand-write a faithful v10 sessions table (carries `plan_snapshot` and
/// `plan_input_count` but NOT `autopilot_mode`), then reopen through
/// `LibsqlStore::open` so `migrate(conn, 10)` runs the `if from < 11` block.
/// Asserts the column is added (NULL by default), round-trips through
/// `SessionPatch`, and that the schema version bumps to 11.
#[tokio::test]
async fn schema_migration_v10_to_v11_adds_autopilot_mode() {
    use libsql::Builder;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("migrate-v11.db");

    // Phase 1: hand-write a v10 sessions table. It has the full v10 column
    // set (including plan_snapshot / plan_input_count) but NO autopilot_mode.
    {
        let db = Builder::new_local(&db_path).build().await.unwrap();
        let conn = db.connect().unwrap();
        conn.execute("CREATE TABLE schema_version (version INTEGER NOT NULL)", ())
            .await
            .unwrap();
        // sessions at v10: full column set EXCEPT autopilot_mode.
        conn.execute(
            "CREATE TABLE sessions (\
               id TEXT PRIMARY KEY, title TEXT, agent TEXT, model TEXT, workdir_hash TEXT,\
               created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,\
               summary TEXT, summary_seq INTEGER, summary_images_json TEXT,\
               handoff_seq INTEGER, handoff_plan TEXT, skill TEXT,\
               task_type TEXT NOT NULL DEFAULT 'parent', requirement TEXT,\
               plan_snapshot TEXT, plan_input_count INTEGER NOT NULL DEFAULT 0)",
            (),
        )
        .await
        .unwrap();
        conn.execute("INSERT INTO schema_version (version) VALUES (10)", ())
            .await
            .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES ('s10', 1, 1)",
            (),
        )
        .await
        .unwrap();
    }

    // Phase 2: reopen — migrate(conn, 10) runs the `if from < 11` block,
    // adding the autopilot_mode column to sessions.
    let store = LibsqlStore::open(&db_path).await.unwrap();

    // (1) The autopilot_mode column now exists on sessions (and is NULL).
    {
        let conn = store.conn().await.unwrap();
        let stmt = conn.prepare("PRAGMA table_info(sessions)").await.unwrap();
        let mut rows = stmt.query(()).await.unwrap();
        let mut found = false;
        while let Some(row) = rows.next().await.unwrap() {
            let name: String = row.get(1).unwrap();
            if name == "autopilot_mode" {
                found = true;
                break;
            }
        }
        assert!(
            found,
            "sessions.autopilot_mode column must exist after v10->v11 migration"
        );
    }

    // (2) Pre-existing row survives; the new column reads back as NULL/None.
    let m0 = store.get_session("s10").await.unwrap().unwrap();
    assert_eq!(m0.id, "s10");
    assert_eq!(
        m0.autopilot_mode, None,
        "v10 row: autopilot_mode reads as NULL after migration"
    );

    // (3) The migrated column round-trips through SessionPatch.
    store
        .update_session(
            "s10",
            &SessionPatch {
                autopilot_mode: Some("ap".into()),
                updated_at: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let m1 = store.get_session("s10").await.unwrap().unwrap();
    assert_eq!(
        m1.autopilot_mode.as_deref(),
        Some("ap"),
        "migrated autopilot_mode round-trips"
    );

    // (4) Schema version bumped to 11.
    {
        let conn = store.conn().await.unwrap();
        let stmt = conn
            .prepare("SELECT version FROM schema_version LIMIT 1")
            .await
            .unwrap();
        let mut rows = stmt.query(()).await.unwrap();
        let r = rows.next().await.unwrap().unwrap();
        let v: i64 = r.get(0).unwrap();
        assert_eq!(v, 11, "schema version must be 11 after v10->v11 migration");
    }
}

async fn index_count(store: &LibsqlStore) -> i64 {
    let conn = store.conn().await.unwrap();
    let stmt = conn
        .prepare(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_subagent_task_id'",
        )
        .await
        .unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    let r = rows.next().await.unwrap().unwrap();
    r.get(0).unwrap()
}

/// Bootstrap creates (and re-creates idempotently) the `task_id` index on
/// `subagent_tasks`: the COMPLETE / CANCEL / get-by-task-id paths filter by
/// `task_id` alone and previously full-scanned the table on every replay /
/// interrupt probe.
#[tokio::test]
async fn bootstrap_creates_subagent_task_id_index() {
    let (dir, store) = fresh().await;
    assert_eq!(
        index_count(&store).await,
        1,
        "idx_subagent_task_id must exist after bootstrap"
    );

    // Reopening the same database (idempotent bootstrap) must not fail or
    // duplicate the index.
    drop(store);
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();
    assert_eq!(
        index_count(&store).await,
        1,
        "idx_subagent_task_id stays singular across reopens"
    );
}
