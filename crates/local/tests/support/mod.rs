//! Fixtures shared by the `local` integration tests.

use std::path::{Path, PathBuf};

use opencoder_store::TursoStore;

/// The per-workdir database the `session` and `todos` commands open for
/// `workdir` (the same path `session_cmd::database_for` resolves).
pub fn database_for(workdir: &Path) -> PathBuf {
    opencoder_core::data_dir_for(workdir).join("opencoder.db")
}

/// The hand-written v3 database of `crates/store/tests/schema_v4_migration.rs`:
/// version row, the v3 `sessions` shape (no `kind`/`task_type`, so this
/// build's list query fails on it) and the v3 `session_inputs`, one session.
const V3_SCHEMA: &str = "\
CREATE TABLE schema_version (version INTEGER NOT NULL); \
CREATE TABLE sessions (\
  id TEXT PRIMARY KEY, title TEXT, agent TEXT, model TEXT, workdir_hash TEXT, \
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, summary TEXT, summary_seq INTEGER, \
  handoff_seq INTEGER, handoff_plan TEXT, skill TEXT\
); \
CREATE TABLE session_inputs (\
  seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL, session_id TEXT NOT NULL, \
  delivery TEXT NOT NULL, prompt TEXT NOT NULL, admitted_seq INTEGER NOT NULL, promoted_seq INTEGER\
); \
INSERT INTO schema_version (version) VALUES (3); \
INSERT INTO sessions (id, created_at, updated_at) VALUES ('OLD1', 1, 1);";

/// Replace the database the CLI opens for `workdir` with a v3 one and return
/// its path. The store is written through a writer of this build and downgraded
/// by hand (every table dropped, the v3 schema written), then released, so
/// nothing owns it when the command under test opens it as a reader.
pub async fn old_schema_store(workdir: &Path) -> PathBuf {
    let database = database_for(workdir);
    std::fs::create_dir_all(database.parent().unwrap()).unwrap();
    let store = TursoStore::open(&database).await.unwrap();
    let conn = store.conn().await.unwrap();
    conn.execute("PRAGMA foreign_keys=OFF", ()).await.unwrap();
    let mut rows = conn
        .query(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            (),
        )
        .await
        .unwrap();
    let mut tables = Vec::new();
    while let Some(row) = rows.next().await.unwrap() {
        tables.push(row.get::<String>(0).unwrap());
    }
    drop(rows);
    // Engine-internal sequence tables (`__turso_internal_*`) go with the
    // AUTOINCREMENT table they belong to; never name them explicitly.
    for table in tables.iter().filter(|table| !table.starts_with("__turso_")) {
        conn.execute(&format!("DROP TABLE IF EXISTS {table}"), ())
            .await
            .unwrap();
    }
    conn.execute_batch(V3_SCHEMA).await.unwrap();
    drop(conn);
    drop(store);
    database
}
