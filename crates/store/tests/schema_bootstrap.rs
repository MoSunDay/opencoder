//! Functional tests for cold-start schema bootstrap behavior on a real
//! on-disk libsql file (WAL semantics exercised truthfully, tempdir-backed).
//!
//! Behavior contracts:
//! - synchronous_is_normal_after_open: the connection ends up with
//!   `synchronous=NORMAL` (1) and `journal_mode=wal`. Order matters: the WAL
//!   switch on a fresh database fsyncs under the *currently active*
//!   synchronous policy, so pragmas must be applied in the safe order or a
//!   cold start pays FULL fsyncs (seconds-minutes inside a ZFS I/O storm).
//! - fresh_open_then_reopen_is_idempotent: fresh open -> one session via the
//!   Store trait -> drop -> two re-opens on the same path. Every re-open
//!   re-runs bootstrap, which must be idempotent: the session survives, the
//!   `schema_version` table still holds exactly one row, and the database
//!   passes `PRAGMA integrity_check`. Running bootstrap inside a single
//!   `BEGIN IMMEDIATE` transaction must not leave nested-transaction debris
//!   (a broken wrapper surfaces as "cannot start a transaction within a
//!   transaction" on the second open).
//! - second_open_on_live_path_succeeds: opening the same path a second time
//!   *while the first store is still alive* - the concurrent-open path whose
//!   `BEGIN IMMEDIATE` queues on the write lock held by busy_timeout.
//! - legacy_tables_without_version_row_converge_on_open: a pre-versioning
//!   database (tables present, no schema_version row) is detected before the
//!   DDL batch and upgraded via a full migrate-from-v0 pass - open succeeds,
//!   the migrated columns + task-type index exist, legacy rows survive with
//!   correct backfill, and a second open converges idempotently.
//! - failed_bootstrap_rolls_back_and_reopens_after_repair: a DDL failure
//!   injected mid-transaction rolls the WHOLE bootstrap back (even the
//!   schema_version table created earlier in the tx is gone again) and leaves
//!   the old state intact, so the database reopens once the defect is
//!   repaired.
//! - checkpoint_gate_existing_path_reopen_converges: the `if existed` gate in
//!   `LibsqlStore::open` (fresh file skips the checkpoint, existing file runs
//!   it) - decision unit-pinned on `should_checkpoint_wal`, here the
//!   existing-path reopen converges with integrity_check ok.

use opencoder_store::{LibsqlStore, SessionMeta, Store, TASK_TYPE_PARENT, TASK_TYPE_SUBAGENT};

/// Read a single-row single-column scalar pragma off a raw connection.
async fn scalar(conn: &libsql::Connection, pragma: &str) -> String {
    let stmt = conn.prepare(pragma).await.unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    let row = rows.next().await.unwrap().expect("pragma row");
    row.get::<String>(0).unwrap()
}

async fn count_schema_version_rows(conn: &libsql::Connection) -> i64 {
    let stmt = conn
        .prepare("SELECT COUNT(*) FROM schema_version")
        .await
        .unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap()
}

/// Contract 1: pragma order invariant holds on the *opened* connection, not
/// just in the PRAGMAS const — synchronous must read back NORMAL (1) and the
/// journal must actually be WAL after a cold open.
#[tokio::test]
async fn synchronous_is_normal_after_open() {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("cold.db")).await.unwrap();
    let conn = store.conn().await.unwrap();

    let synchronous: i64 = {
        let stmt = conn.prepare("PRAGMA synchronous").await.unwrap();
        let mut rows = stmt.query(()).await.unwrap();
        rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap()
    };
    assert_eq!(synchronous, 1, "synchronous must be NORMAL (1) after open");

    assert_eq!(
        scalar(&conn, "PRAGMA journal_mode").await.to_lowercase(),
        "wal",
        "journal_mode must be wal after open"
    );
}

/// Contract 2: bootstrap re-runs (two re-opens on the same path) stay
/// idempotent and leave a healthy, single-versioned database.
#[tokio::test]
async fn fresh_open_then_reopen_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reopen.db");

    // Fresh open + one durable session through the Store trait.
    {
        let store = LibsqlStore::open(&path).await.unwrap();
        store
            .create_session(&SessionMeta {
                id: "boot-s1".into(),
                created_at: 1,
                updated_at: 1,
                ..Default::default()
            })
            .await
            .unwrap();
    }

    // Two re-opens on the same path: each re-runs the (single-transaction)
    // bootstrap. A nested BEGIN would fail the whole open loudly.
    for reopen in 1..=2 {
        let store = LibsqlStore::open(&path).await.unwrap();
        let conn = store.conn().await.unwrap();

        let session = store.get_session("boot-s1").await.unwrap();
        assert!(session.is_some(), "session must survive re-open #{reopen}");
        assert_eq!(
            count_schema_version_rows(&conn).await,
            1,
            "schema_version must hold exactly one row after re-open #{reopen}"
        );
        assert_eq!(
            scalar(&conn, "PRAGMA integrity_check").await,
            "ok",
            "integrity_check must be ok after re-open #{reopen}"
        );
    }
}

/// Contract 3: concurrent-open path — bootstrap twice against the same file
/// with the first store still alive. The second bootstrap's BEGIN IMMEDIATE
/// must queue on the write lock (busy_timeout) instead of erroring, proving
/// the single-transaction wrapper leaves no transaction state behind.
#[tokio::test]
async fn second_open_on_live_path_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("live.db");

    let first = LibsqlStore::open(&path).await.unwrap();
    let second = LibsqlStore::open(&path).await.unwrap();

    assert!(
        first.get_session("none").await.unwrap().is_none(),
        "baseline read on the first store"
    );
    second
        .create_session(&SessionMeta {
            id: "live-s1".into(),
            created_at: 1,
            updated_at: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    let seen = first.get_session("live-s1").await.unwrap();
    assert!(seen.is_some(), "both handles must see the same database");

    let conn = first.conn().await.unwrap();
    assert_eq!(
        count_schema_version_rows(&conn).await,
        1,
        "double bootstrap must not duplicate the version row"
    );
}

// ===========================================================================
// Bug 04: legacy shape, version row absent
// ===========================================================================
// Databases written before schema_version tracking carry tables but no version
// row. Bootstrap's CREATE batch is a no-op on those tables, so without a
// migrate pass the stale `sessions` shape survives and the post-migration
// `idx_sessions_task_type` fails - aborting the single bootstrap transaction
// and re-failing on every later open (permanently unopenable database).

async fn raw_open(db_path: &std::path::Path) -> libsql::Connection {
    let db = libsql::Builder::new_local(db_path).build().await.unwrap();
    db.connect().unwrap()
}

/// Mirror of the crate-internal `column_exists`: reads `PRAGMA table_info`,
/// where the column name lives at result index 1.
async fn has_column(conn: &libsql::Connection, table: &str, column: &str) -> bool {
    let stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .await
        .unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    while let Some(row) = rows.next().await.unwrap() {
        if row.get::<String>(1).unwrap() == column {
            return true;
        }
    }
    false
}

async fn table_named(conn: &libsql::Connection, name: &str) -> bool {
    let stmt = conn
        .prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1")
        .await
        .unwrap();
    let mut rows = stmt.query(libsql::params![name]).await.unwrap();
    rows.next().await.unwrap().is_some()
}

/// `Some(v)` from the version row; caller ensures the table exists.
async fn version_of(conn: &libsql::Connection) -> i64 {
    let stmt = conn
        .prepare("SELECT version FROM schema_version LIMIT 1")
        .await
        .unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap()
}

/// Hand-write a pre-versioning database: the five core tables in their old
/// shapes (sessions without `task_type`, session_events without `sse_kind`,
/// session_inputs without `images_json`/`display_text`/`recorded`) and NO
/// schema_version table at all. A parent + subagent child pair exercises the
/// v5 backfill during the repair.
async fn seed_legacy_db(db_path: &std::path::Path, inputs_without_promoted_seq: bool) {
    let conn = raw_open(db_path).await;
    conn.execute(
        "CREATE TABLE sessions (\
           id TEXT PRIMARY KEY, title TEXT, agent TEXT, model TEXT, workdir_hash TEXT,\
           created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, summary TEXT, summary_seq INTEGER)",
        (),
    )
    .await
    .unwrap();
    conn.execute(
        "CREATE TABLE messages (\
           seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL, session_id TEXT NOT NULL,\
           role TEXT NOT NULL, agent TEXT, model TEXT, blocks_json TEXT NOT NULL,\
           usage_json TEXT NOT NULL, created_at INTEGER NOT NULL)",
        (),
    )
    .await
    .unwrap();
    let promoted = if inputs_without_promoted_seq {
        ""
    } else {
        ", promoted_seq INTEGER"
    };
    conn.execute(
        &format!(
            "CREATE TABLE session_inputs (\
               seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL, session_id TEXT NOT NULL,\
               delivery TEXT NOT NULL, prompt TEXT NOT NULL, admitted_seq INTEGER NOT NULL{promoted})"
        ),
        (),
    )
    .await
    .unwrap();
    conn.execute(
        "CREATE TABLE session_events (\
           seq INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, type TEXT NOT NULL,\
           payload_json TEXT NOT NULL, ts INTEGER NOT NULL)",
        (),
    )
    .await
    .unwrap();
    conn.execute(
        "CREATE TABLE subagent_tasks (\
           seq INTEGER PRIMARY KEY AUTOINCREMENT, task_id TEXT NOT NULL,\
           parent_session_id TEXT NOT NULL, child_session_id TEXT NOT NULL,\
           parent_message_id TEXT, agent TEXT NOT NULL, prompt TEXT NOT NULL,\
           result TEXT, status TEXT NOT NULL, ok INTEGER, started_at INTEGER NOT NULL,\
           completed_at INTEGER)",
        (),
    )
    .await
    .unwrap();
    for id in ["parent", "child"] {
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES (?1, 1, 1)",
            libsql::params![id],
        )
        .await
        .unwrap();
    }
    conn.execute(
        "INSERT INTO subagent_tasks (task_id, parent_session_id, child_session_id, agent, prompt, status, started_at)\
         VALUES ('t1', 'parent', 'child', 'explore', 'p', 'done', 1)",
        (),
    )
    .await
    .unwrap();
    conn.execute(
        "INSERT INTO messages (id, session_id, role, blocks_json, usage_json, created_at)\
         VALUES ('m1', 'parent', 'user', '[]', '{}', 1)",
        (),
    )
    .await
    .unwrap();
}

/// Bug 04 regression: tables present but version row absent must converge on
/// open - the legacy shape is detected before the DDL batch and upgraded
/// through a full `migrate(0)` pass (safe because every migrate step is
/// `IF NOT EXISTS` / `add_column_if_absent` / converging backfill), instead
/// of being version-stamped and then dying on the task-type index.
#[tokio::test]
async fn legacy_tables_without_version_row_converge_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    seed_legacy_db(&path, false).await;

    // First open: previously failed inside the bootstrap transaction with
    // "no such column: task_type" (index over the stale sessions shape).
    let store = LibsqlStore::open(&path).await.unwrap();
    let conn = store.conn().await.unwrap();

    assert_eq!(
        version_of(&conn).await,
        18,
        "version row must be stamped at the latest version"
    );
    for (table, column) in [
        ("sessions", "task_type"),
        ("sessions", "handoff_seq"),
        ("sessions", "autopilot_mode"),
        ("session_events", "sse_kind"),
        ("session_inputs", "images_json"),
        ("session_inputs", "recorded"),
    ] {
        assert!(
            has_column(&conn, table, column).await,
            "{table}.{column} must exist after the legacy repair"
        );
    }
    let stmt = conn
        .prepare("SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_sessions_task_type'")
        .await
        .unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    assert_eq!(
        rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
        1,
        "idx_sessions_task_type must be built"
    );

    // Legacy data survives; the subagent child is backfilled, the parent keeps
    // the column default.
    let parent = store.get_session("parent").await.unwrap().unwrap();
    assert_eq!(parent.task_type.as_deref(), Some(TASK_TYPE_PARENT));
    let child = store.get_session("child").await.unwrap().unwrap();
    assert_eq!(child.task_type.as_deref(), Some(TASK_TYPE_SUBAGENT));
    let stmt = conn.prepare("SELECT COUNT(*) FROM messages").await.unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    assert_eq!(
        rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
        1
    );

    // Second open is idempotent: same version, no duplicate row, healthy db.
    drop(store);
    let store = LibsqlStore::open(&path).await.unwrap();
    let conn = store.conn().await.unwrap();
    assert_eq!(
        version_of(&conn).await,
        18,
        "re-open must not move the version"
    );
    assert_eq!(count_schema_version_rows(&conn).await, 1);
    assert!(has_column(&conn, "sessions", "task_type").await);
    assert_eq!(scalar(&conn, "PRAGMA integrity_check").await, "ok");
}

/// Bug 04, rollback side: a DDL failure mid-bootstrap rolls the WHOLE
/// transaction back and leaves the old state intact, so the database is
/// reopenable once the defect is repaired.
///
/// Injection note: the "pre-create a same-name object" route is impossible -
/// every bootstrap DDL is `IF NOT EXISTS` or `add_column_if_absent`-guarded,
/// so a colliding name is silently skipped. The deterministic injection point
/// is a shape mismatch a later statement cannot tolerate: a legacy
/// session_inputs lacking `promoted_seq` fails `idx_inputs_pending` creation
/// several statements AFTER `schema_version` and `todo_workflows` were
/// created - which is exactly what the rollback assertions below witness.
#[tokio::test]
async fn failed_bootstrap_rolls_back_and_reopens_after_repair() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rollback.db");
    seed_legacy_db(&path, true).await;

    let first = LibsqlStore::open(&path).await;
    assert!(
        first.is_err(),
        "index over the missing promoted_seq column must fail the open"
    );

    // Whole-transaction rollback: the db is exactly as hand-written. Nothing
    // the failed bootstrap created (schema_version, todo_workflows, ...)
    // survived, no migration column leaked, and the legacy row is untouched.
    let conn = raw_open(&path).await;
    assert!(
        !table_named(&conn, "schema_version").await,
        "the CREATE earlier in the tx must roll back"
    );
    assert!(
        !table_named(&conn, "todo_workflows").await,
        "mid-tx created tables must roll back"
    );
    assert!(
        !has_column(&conn, "sessions", "task_type").await,
        "no migration DDL may leak"
    );
    let stmt = conn.prepare("SELECT COUNT(*) FROM sessions").await.unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    assert_eq!(
        rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
        2
    );

    // Repair the injected defect; the legacy path then converges normally.
    conn.execute(
        "ALTER TABLE session_inputs ADD COLUMN promoted_seq INTEGER",
        (),
    )
    .await
    .unwrap();
    drop(conn);

    let store = LibsqlStore::open(&path).await.unwrap();
    let conn = store.conn().await.unwrap();
    assert_eq!(version_of(&conn).await, 18);
    assert!(has_column(&conn, "sessions", "task_type").await);
    assert!(store.get_session("parent").await.unwrap().is_some());
    assert_eq!(scalar(&conn, "PRAGMA integrity_check").await, "ok");
}

/// Bug 10: `open`'s `if existed` checkpoint gate. The decision itself is
/// unit-pinned on `should_checkpoint_wal`; this proves the gate's inputs in
/// situ and that an existing-path reopen (checkpoint branch taken) converges
/// with a healthy database.
#[tokio::test]
async fn checkpoint_gate_existing_path_reopen_converges() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gate.db");

    // Fresh file: gate input false - open skips the checkpoint.
    assert!(
        !path.exists(),
        "gate input must be false before the first open"
    );
    {
        let store = LibsqlStore::open(&path).await.unwrap();
        store
            .create_session(&SessionMeta {
                id: "gate-1".into(),
                created_at: 1,
                updated_at: 1,
                ..Default::default()
            })
            .await
            .unwrap();
    }
    // The file now pre-exists: the reopen below takes the checkpoint branch.
    assert!(path.exists(), "gate input must be true for the reopen");

    let store = LibsqlStore::open(&path).await.unwrap();
    let conn = store.conn().await.unwrap();
    assert!(store.get_session("gate-1").await.unwrap().is_some());
    assert_eq!(count_schema_version_rows(&conn).await, 1);
    assert_eq!(scalar(&conn, "PRAGMA integrity_check").await, "ok");
}
