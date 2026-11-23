//! Requirement and supporting-index migrations.

use std::path::Path;

use opencoder_store::{SessionPatch, Store, TursoStore};

// ===========================================================================
// v7 -> v8 migration: the `sessions.requirement` column.
// ===========================================================================

/// Hand-write a faithful v7 sessions table (carries `summary_images_json` but
/// NOT `requirement`), then reopen through `TursoStore::open` so
/// `migrate(conn, 7)` runs the `if from < 8` block. Asserts the column is
/// added (NULL by default), round-trips through `SessionPatch`, and that the
/// schema version bumps to the latest (12).
#[tokio::test]
async fn schema_migration_v7_to_v8_adds_requirement() {
    use turso::Builder;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("migrate-v8.db");

    // Phase 1: hand-write a v7 sessions table. It has the full v7 column set
    // (including summary_images_json) but NO requirement column.
    {
        let db = Builder::new_local(db_path.to_str().expect("UTF-8 database path"))
            .experimental_multiprocess_wal(true)
            .build()
            .await
            .unwrap();
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
    let store = TursoStore::open(&db_path).await.unwrap();

    // (1) The requirement column now exists on sessions (and is NULL).
    {
        let conn = store.conn().await.unwrap();
        let mut stmt = conn.prepare("PRAGMA table_info(sessions)").await.unwrap();
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
        let mut stmt = conn
            .prepare("SELECT version FROM schema_version LIMIT 1")
            .await
            .unwrap();
        let mut rows = stmt.query(()).await.unwrap();
        let r = rows.next().await.unwrap().unwrap();
        let v: i64 = r.get(0).unwrap();
        assert_eq!(
            v,
            opencoder_store::turso_store::schema_watermark(),
            "schema version must be current watermark after v7->v8 migration"
        );
    }
}

// ===========================================================================
// Latest-schema convergence of `sessions`: the session-scoped autopilot mode
// column was removed together with its feature (v40 drops it). Pre-removal
// databases must never gain or keep it, while their rows survive the upgrade.
// ===========================================================================

/// Column names of `sessions`, in declaration order.
async fn session_columns(store: &TursoStore) -> Vec<String> {
    let conn = store.conn().await.unwrap();
    let mut stmt = conn.prepare("PRAGMA table_info(sessions)").await.unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    let mut names = Vec::new();
    while let Some(row) = rows.next().await.unwrap() {
        let name: String = row.get(1).unwrap();
        names.push(name);
    }
    names
}

/// The single recorded schema watermark row.
async fn recorded_schema_version(store: &TursoStore) -> i64 {
    let conn = store.conn().await.unwrap();
    let mut stmt = conn
        .prepare("SELECT version FROM schema_version LIMIT 1")
        .await
        .unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    let r = rows.next().await.unwrap().unwrap();
    r.get(0).unwrap()
}

/// Hand-write a legacy database holding only `schema_version` and a `sessions`
/// table built from `sessions_ddl`: stamped at `version` and seeded with one
/// row (`row_id`, created 1, updated 1). Enough for `TursoStore::open` to run
/// the migration chain followed by the additive bootstrap batch.
async fn write_legacy_sessions_db(db_path: &Path, version: i64, sessions_ddl: &str, row_id: &str) {
    use turso::Builder;

    let db = Builder::new_local(db_path.to_str().expect("UTF-8 database path"))
        .experimental_multiprocess_wal(true)
        .build()
        .await
        .unwrap();
    let conn = db.connect().unwrap();
    conn.execute("CREATE TABLE schema_version (version INTEGER NOT NULL)", ())
        .await
        .unwrap();
    conn.execute(sessions_ddl, ()).await.unwrap();
    conn.execute(
        "INSERT INTO schema_version (version) VALUES (?1)",
        turso::params![version],
    )
    .await
    .unwrap();
    conn.execute(
        "INSERT INTO sessions (id, created_at, updated_at) VALUES (?1, 1, 1)",
        turso::params![row_id.to_string()],
    )
    .await
    .unwrap();
}

/// sessions at v10: full column set with `plan_snapshot` / `plan_input_count`,
/// predating the removed column.
const V10_SESSIONS_DDL: &str = "CREATE TABLE sessions (\
     id TEXT PRIMARY KEY, title TEXT, agent TEXT, model TEXT, workdir_hash TEXT,\
     created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,\
     summary TEXT, summary_seq INTEGER, summary_images_json TEXT,\
     handoff_seq INTEGER, handoff_plan TEXT, skill TEXT,\
     task_type TEXT NOT NULL DEFAULT 'parent', requirement TEXT,\
     plan_snapshot TEXT, plan_input_count INTEGER NOT NULL DEFAULT 0)";

/// A v10 database upgraded through the full chain must NOT regain the removed
/// column; pre-existing rows survive and the schema version reaches the
/// watermark.
#[tokio::test]
async fn schema_migration_v10_to_latest_preserves_rows_without_removed_column() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("migrate-v10-latest.db");
    write_legacy_sessions_db(&db_path, 10, V10_SESSIONS_DDL, "s10").await;

    // Reopen: migrate(conn, 10) runs the whole chain up to the watermark.
    let store = TursoStore::open(&db_path).await.unwrap();

    // (1) The removed column is absent from sessions.
    assert!(
        !session_columns(&store)
            .await
            .iter()
            .any(|c| c == "autopilot_mode"),
        "sessions.autopilot_mode must stay absent after v10->latest migration"
    );

    // (2) Pre-existing row survives the migration.
    let m0 = store.get_session("s10").await.unwrap().unwrap();
    assert_eq!(m0.id, "s10");

    // (3) Schema version bumped to the watermark.
    assert_eq!(
        recorded_schema_version(&store).await,
        opencoder_store::turso_store::schema_watermark(),
        "schema version must be current watermark after v10->latest migration"
    );
}

/// sessions at v39: the current column set plus the since-removed column.
const V39_SESSIONS_DDL: &str = "CREATE TABLE sessions (\
     id TEXT PRIMARY KEY, title TEXT, agent TEXT, model TEXT, workdir_hash TEXT,\
     created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,\
     summary TEXT, summary_seq INTEGER, summary_images_json TEXT,\
     handoff_seq INTEGER, handoff_plan TEXT, skill TEXT,\
     task_type TEXT NOT NULL DEFAULT 'parent', requirement TEXT,\
     plan_snapshot TEXT, plan_input_count INTEGER NOT NULL DEFAULT 0,\
     harness_runtime TEXT, kind TEXT, autopilot_mode TEXT)";

/// A v39 database still carries the removed column; the v40 step must drop it
/// while preserving rows and stamping the new watermark.
#[tokio::test]
async fn schema_migration_v39_to_v40_drops_removed_autopilot_column() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("migrate-v39-v40.db");
    write_legacy_sessions_db(&db_path, 39, V39_SESSIONS_DDL, "s39").await;

    // Reopen: migrate(conn, 39) runs the `if from < 40` block.
    let store = TursoStore::open(&db_path).await.unwrap();

    // (1) The removed column is gone.
    assert!(
        !session_columns(&store)
            .await
            .iter()
            .any(|c| c == "autopilot_mode"),
        "sessions.autopilot_mode must be dropped by the v40 migration"
    );

    // (2) Pre-existing row survives the migration.
    let m0 = store.get_session("s39").await.unwrap().unwrap();
    assert_eq!(m0.id, "s39");

    // (3) Schema version bumped to the watermark.
    assert_eq!(
        recorded_schema_version(&store).await,
        opencoder_store::turso_store::schema_watermark(),
        "schema version must be current watermark after v39->v40 migration"
    );
}
