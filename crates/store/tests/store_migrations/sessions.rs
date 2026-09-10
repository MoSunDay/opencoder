//! Requirement, autopilot, and supporting-index migrations.

use opencoder_store::{LibsqlStore, SessionPatch, Store};

// ===========================================================================
// v7 -> v8 migration: the `sessions.requirement` column.
// ===========================================================================

/// Hand-write a faithful v7 sessions table (carries `summary_images_json` but
/// NOT `requirement`), then reopen through `LibsqlStore::open` so
/// `migrate(conn, 7)` runs the `if from < 8` block. Asserts the column is
/// added (NULL by default), round-trips through `SessionPatch`, and that the
/// schema version bumps to the latest (12).
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
        assert_eq!(
            v, 24,
            "schema version must be latest (24) after v7->v8 migration"
        );
    }
}

// ===========================================================================
// v10 -> v11 migration: the `sessions.autopilot_mode` column.
// ===========================================================================

/// Hand-write a faithful v10 sessions table (carries `plan_snapshot` and
/// `plan_input_count` but NOT `autopilot_mode`), then reopen through
/// `LibsqlStore::open` so `migrate(conn, 10)` runs the `if from < 11` block.
/// Asserts the column is added (NULL by default), round-trips through
/// `SessionPatch`, and that the schema version bumps to the latest (12).
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

    // (4) Schema version bumped to 12.
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
            v, 24,
            "schema version must be latest (24) after v10->v11 migration"
        );
    }
}
