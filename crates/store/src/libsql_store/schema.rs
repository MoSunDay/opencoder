use anyhow::{Context, Result};
use libsql::Connection;

const SCHEMA_VERSION: i64 = 10;

const PRAGMAS: &[&str] = &[
    "PRAGMA busy_timeout=30000",
    "PRAGMA journal_mode=WAL",
    "PRAGMA synchronous=NORMAL",
    "PRAGMA foreign_keys=ON",
    "PRAGMA cache_size=-65536",
    "PRAGMA wal_autocheckpoint=1000",
];

const CREATE_SCHEMA_VERSION: &str =
    "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL)";
const CREATE_SESSIONS: &str = "\
CREATE TABLE IF NOT EXISTS sessions (
  id           TEXT PRIMARY KEY,
  title        TEXT,
  agent        TEXT,
  model        TEXT,
  workdir_hash TEXT,
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL,
  summary      TEXT,
  summary_seq      INTEGER,
  summary_images_json TEXT,
  handoff_seq  INTEGER,
  handoff_plan TEXT,
  skill        TEXT,
  task_type    TEXT NOT NULL DEFAULT 'parent',
  requirement  TEXT,
  plan_snapshot TEXT,
  plan_input_count INTEGER NOT NULL DEFAULT 0
)";
const CREATE_MESSAGES: &str = "\
CREATE TABLE IF NOT EXISTS messages (
  seq         INTEGER PRIMARY KEY AUTOINCREMENT,
  id          TEXT NOT NULL,
  session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  role        TEXT NOT NULL,
  agent       TEXT,
  model       TEXT,
  blocks_json TEXT NOT NULL,
  usage_json  TEXT NOT NULL,
  created_at  INTEGER NOT NULL,
  synthetic   INTEGER NOT NULL DEFAULT 0,
  mode        TEXT,
  summary     INTEGER NOT NULL DEFAULT 0
)";
const CREATE_INPUTS: &str = "\
CREATE TABLE IF NOT EXISTS session_inputs (
  seq          INTEGER PRIMARY KEY AUTOINCREMENT,
  id           TEXT NOT NULL,
  session_id   TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  delivery     TEXT NOT NULL,
  prompt       TEXT NOT NULL,
  images_json  TEXT NOT NULL DEFAULT '[]',
  display_text TEXT,
  admitted_seq INTEGER NOT NULL,
  promoted_seq INTEGER,
  recorded     INTEGER NOT NULL DEFAULT 0
)";
const CREATE_EVENTS: &str = "\
CREATE TABLE IF NOT EXISTS session_events (
  seq          INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id   TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  type         TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  sse_kind     TEXT,
  ts           INTEGER NOT NULL
)";
const CREATE_SUBAGENT_TASKS: &str = "\
CREATE TABLE IF NOT EXISTS subagent_tasks (
  seq               INTEGER PRIMARY KEY AUTOINCREMENT,
  task_id           TEXT NOT NULL,
  parent_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  child_session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  parent_message_id TEXT,
  agent             TEXT NOT NULL,
  prompt            TEXT NOT NULL,
  result            TEXT,
  status            TEXT NOT NULL,
  ok                INTEGER,
  started_at        INTEGER NOT NULL,
  completed_at      INTEGER
)";
const CREATE_TODO_WORKFLOWS: &str = "\
CREATE TABLE IF NOT EXISTS todo_workflows (
  id TEXT PRIMARY KEY,
  parent_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  status TEXT NOT NULL,
  spec_json TEXT NOT NULL,
  state_json TEXT NOT NULL,
  generation INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  terminal_reason TEXT
)";
const CREATE_TODO_ITEMS: &str = "\
CREATE TABLE IF NOT EXISTS todo_items (
  workflow_id TEXT NOT NULL REFERENCES todo_workflows(id) ON DELETE CASCADE,
  todo_id TEXT NOT NULL,
  ordinal INTEGER NOT NULL,
  status TEXT NOT NULL,
  attempt INTEGER NOT NULL,
  active_session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
  session_history_json TEXT NOT NULL DEFAULT '[]',
  result_json TEXT,
  last_error TEXT,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (workflow_id, todo_id)
)";
const CREATE_TODO_EVENTS: &str = "\
CREATE TABLE IF NOT EXISTS todo_events (
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  workflow_id TEXT NOT NULL REFERENCES todo_workflows(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  ts INTEGER NOT NULL
)";
const CREATE_INDEX_MSG: &str =
    "CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, seq)";
const CREATE_INDEX_IN: &str = "CREATE INDEX IF NOT EXISTS idx_inputs_pending ON session_inputs(session_id, promoted_seq, delivery, admitted_seq)";
const CREATE_INDEX_EV: &str =
    "CREATE INDEX IF NOT EXISTS idx_events_session ON session_events(session_id, seq)";
const CREATE_INDEX_SA_PARENT: &str =
    "CREATE INDEX IF NOT EXISTS idx_subagent_parent ON subagent_tasks(parent_session_id, seq)";
const CREATE_INDEX_SA_CHILD: &str =
    "CREATE INDEX IF NOT EXISTS idx_subagent_child ON subagent_tasks(child_session_id)";
const CREATE_INDEX_SESSION_TASK_TYPE: &str =
    "CREATE INDEX IF NOT EXISTS idx_sessions_task_type ON sessions(task_type)";
const CREATE_INDEX_TODO_STATUS: &str =
    "CREATE INDEX IF NOT EXISTS idx_todo_workflows_status ON todo_workflows(status, updated_at)";
const CREATE_INDEX_TODO_EVENTS: &str =
    "CREATE INDEX IF NOT EXISTS idx_todo_events_workflow ON todo_events(workflow_id, seq)";

/// Apply WAL + safety pragmas to a single connection. Cheap to call per-acquire.
///
/// Uses `query` (not `execute`) because some pragmas (e.g. `journal_mode=WAL`)
/// return a row, which libsql's `execute` treats as an error. Draining the
/// rows works for both row-returning and empty pragmas.
pub async fn apply_connection_pragmas(conn: &Connection) -> Result<()> {
    for p in PRAGMAS {
        let stmt = conn
            .prepare(p)
            .await
            .with_context(|| format!("prepare pragma: {p}"))?;
        let mut rows = stmt
            .query(())
            .await
            .with_context(|| format!("pragma: {p}"))?;
        while rows.next().await?.is_some() {
            // drain
        }
    }
    Ok(())
}

/// Best-effort WAL checkpoint. Passively merges the WAL file back into the
/// main database so it doesn't grow unbounded. Safe to call after schema
/// bootstrap on open. Errors are non-fatal (WAL will auto-checkpoint via
/// `wal_autocheckpoint` when it reaches the configured page threshold).
pub async fn checkpoint_wal(conn: &Connection) -> Result<()> {
    let stmt = conn.prepare("PRAGMA wal_checkpoint(PASSIVE)").await?;
    let mut rows = stmt.query(()).await?;
    while rows.next().await?.is_some() {
        // drain checkpoint result rows
    }
    Ok(())
}

/// Create all tables if absent, run incremental migrations, and record the
/// schema version. Idempotent: safe on fresh and existing databases. Because
/// the `CREATE TABLE` statements carry the full latest schema while
/// `schema_version` may record a stale older version, migrations guard every
/// `ADD COLUMN` via `add_column_if_absent`, so re-running never fails with
/// `duplicate column name`.
pub async fn bootstrap(conn: &Connection) -> Result<()> {
    conn.execute(CREATE_SCHEMA_VERSION, ()).await?;
    conn.execute(CREATE_SESSIONS, ()).await?;
    conn.execute(CREATE_MESSAGES, ()).await?;
    conn.execute(CREATE_INPUTS, ()).await?;
    conn.execute(CREATE_EVENTS, ()).await?;
    conn.execute(CREATE_SUBAGENT_TASKS, ()).await?;
    conn.execute(CREATE_TODO_WORKFLOWS, ()).await?;
    conn.execute(CREATE_TODO_ITEMS, ()).await?;
    conn.execute(CREATE_TODO_EVENTS, ()).await?;
    conn.execute(CREATE_INDEX_MSG, ()).await?;
    conn.execute(CREATE_INDEX_IN, ()).await?;
    conn.execute(CREATE_INDEX_EV, ()).await?;
    conn.execute(CREATE_INDEX_SA_PARENT, ()).await?;
    conn.execute(CREATE_INDEX_SA_CHILD, ()).await?;

    // Incremental migrations: only run when upgrading from a prior version.
    // Fresh databases (version None) already have the full schema from the
    // CREATE TABLE statements above, so migrations are skipped for them.
    let current = current_version(conn).await?;
    if let Some(prev) = current {
        if prev < SCHEMA_VERSION {
            migrate(conn, prev).await?;
            set_version(conn, SCHEMA_VERSION).await?;
        }
    } else {
        set_version(conn, SCHEMA_VERSION).await?;
    }
    // The task_type index depends on a column that only physically exists in
    // fresh databases (via CREATE TABLE) or after the v5 migration adds it for
    // older databases, so it must run AFTER `migrate` rather than in the
    // pre-migration index batch above.
    conn.execute(CREATE_INDEX_SESSION_TASK_TYPE, ()).await?;
    conn.execute(CREATE_INDEX_TODO_STATUS, ()).await?;
    conn.execute(CREATE_INDEX_TODO_EVENTS, ()).await?;
    Ok(())
}

/// Run incremental schema migrations from `from` up to the current version.
///
/// Migrations are idempotent: each `ALTER TABLE ... ADD COLUMN` is guarded by
/// `add_column_if_absent`, which inspects `PRAGMA table_info` and skips the
/// column when it is already present. This is important because `bootstrap`
/// always runs the `CREATE TABLE` statements carrying the *full latest* schema,
/// so a table can already physically carry a column even when `schema_version`
/// records a stale older version (e.g. a database whose schema_version row was
/// left behind at 1 while the tables were recreated at the latest shape). A
/// bare `ADD COLUMN` would fail with `duplicate column name` in that case;
/// guarding the ALTER makes re-migration safe regardless of the
/// CREATE-TABLE-vs-stale-version disagreement.
async fn migrate(conn: &Connection, from: i64) -> Result<()> {
    if from < 2 {
        // v2: add sse_kind column to session_events for lossless event-kind
        // replay. The column is nullable so existing rows stay valid.
        add_column_if_absent(conn, "session_events", "sse_kind", "TEXT").await?;
    }
    if from < 3 {
        // v3: plan→act handoff boundary + active skill on sessions, so resume
        // can reconstruct the post-handoff focused transcript and the active
        // skill across restarts. All nullable so existing rows stay valid.
        add_column_if_absent(conn, "sessions", "handoff_seq", "INTEGER").await?;
        add_column_if_absent(conn, "sessions", "handoff_plan", "TEXT").await?;
        add_column_if_absent(conn, "sessions", "skill", "TEXT").await?;
    }
    if from < 4 {
        // v4: image attachments on session inputs (multimodal prompts). The
        // column is a JSON array of data URIs, defaulting to an empty array so
        // existing plain-text rows stay valid. NOT NULL + DEFAULT keeps the
        // invariant that the column is always readable as JSON.
        add_column_if_absent(
            conn,
            "session_inputs",
            "images_json",
            "TEXT NOT NULL DEFAULT '[]'",
        )
        .await?;
    }
    if from < 5 {
        // v5: task_type column on sessions distinguishes parent (top-level)
        // sessions from subagent child sessions. NOT NULL with a default of
        // 'parent' so existing rows are valid parents. Backfill any rows that
        // are already linked as subagent children, then create the filter
        // index. (CREATE TABLE already carries the column for fresh DBs, so
        // add_column_if_absent keeps this idempotent.)
        add_column_if_absent(
            conn,
            "sessions",
            "task_type",
            "TEXT NOT NULL DEFAULT 'parent'",
        )
        .await?;
        conn.execute(
            "UPDATE sessions SET task_type = 'subagent' WHERE id IN (SELECT child_session_id FROM subagent_tasks)",
            (),
        )
        .await
        .context("backfill task_type")?;
        conn.execute(CREATE_INDEX_SESSION_TASK_TYPE, ()).await?;
    }
    if from < 6 {
        // v6: display-only text on session inputs, preserving the verbatim
        // original (which may contain the `$skill` token) so the TUI queue/
        // steer panel can restore it after resume/reload. `prompt` keeps the
        // clean token-stripped text that the LLM consumes; this column is
        // never fed to the LLM. Nullable so pre-existing rows stay valid —
        // old rows keep NULL and display layers fall back to `prompt`.
        add_column_if_absent(conn, "session_inputs", "display_text", "TEXT").await?;
    }
    if from < 7 {
        // v7: image URLs preserved across compaction, persisted as
        // `summary_images_json` so resume can rebuild the synthetic
        // summary message without reloading the soft-deleted
        // compacted head. Nullable so existing rows stay valid.
        add_column_if_absent(conn, "sessions", "summary_images_json", "TEXT").await?;
    }
    if from < 8 {
        // v8: requirement column on sessions, persisting the task description
        // text edited via the /requirement slash command so it survives resume.
        add_column_if_absent(conn, "sessions", "requirement", "TEXT").await?;
    }
    if from < 9 {
        conn.execute(CREATE_TODO_WORKFLOWS, ()).await?;
        conn.execute(CREATE_TODO_ITEMS, ()).await?;
        conn.execute(CREATE_TODO_EVENTS, ()).await?;
        conn.execute(CREATE_INDEX_TODO_STATUS, ()).await?;
        conn.execute(CREATE_INDEX_TODO_EVENTS, ()).await?;
    }
    if from < 10 {
        // v10: plan-phase persistence. `plan_snapshot` preserves the final
        // plan text across compaction so plan->act handoff still finds it;
        // `plan_input_count` re-arms plan-phase affordances after resume.
        add_column_if_absent(conn, "sessions", "plan_snapshot", "TEXT").await?;
        add_column_if_absent(
            conn,
            "sessions",
            "plan_input_count",
            "INTEGER NOT NULL DEFAULT 0",
        )
        .await?;
    }
    if from < 10 {
        // v10: `recorded` marks a promoted input as durably consumed (written
        // into the transcript or applied as a control command). A promoted row
        // with recorded=0 is an orphan (crash / hard-cancel between promote
        // and consume) that `recover_orphan_inputs` can flip back to pending.
        add_column_if_absent(
            conn,
            "session_inputs",
            "recorded",
            "INTEGER NOT NULL DEFAULT 0",
        )
        .await?;
        // One-time backfill: rows already promoted when the column lands
        // predate the marker and are historical audit rows already reflected
        // in the transcript, so treat them as consumed. Pending rows keep
        // recorded=0.
        conn.execute(
            "UPDATE session_inputs SET recorded = 1 WHERE promoted_seq IS NOT NULL AND recorded = 0",
            (),
        )
        .await
        .context("backfill recorded")?;
    }
    Ok(())
}
/// Return `true` if `table` has a column named `column`.
///
/// Inspects `PRAGMA table_info(<table>)`, where the column-name lives at result
/// index 1. `table` and `column` are code-controlled literals, so interpolating
/// them into the SQL is safe.
async fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let stmt = conn.prepare(&format!("PRAGMA table_info({table})")).await?;
    let mut rows = stmt.query(()).await?;
    while let Some(row) = rows.next().await? {
        let name: String = row.get::<String>(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `ALTER TABLE {table} ADD COLUMN {column} {type_def}`, but a no-op when the
/// column already exists. `table`, `column`, and `type_def` are code-controlled
/// literals (not user input), so the format! interpolation is intentional.
async fn add_column_if_absent(
    conn: &Connection,
    table: &str,
    column: &str,
    type_def: &str,
) -> Result<()> {
    if column_exists(conn, table, column).await? {
        return Ok(());
    }
    conn.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {column} {type_def}"),
        (),
    )
    .await
    .with_context(|| format!("add column {table}.{column}"))?;
    Ok(())
}

pub async fn current_version(conn: &Connection) -> Result<Option<i64>> {
    let stmt = conn
        .prepare("SELECT version FROM schema_version LIMIT 1")
        .await?;
    let mut rows = stmt.query(()).await?;
    if let Some(row) = rows.next().await? {
        Ok(Some(row.get::<i64>(0)?))
    } else {
        Ok(None)
    }
}

async fn set_version(conn: &Connection, version: i64) -> Result<()> {
    // Wrap DELETE + INSERT in a single transaction so a crash between them
    // cannot leave schema_version empty (which would trigger a spurious full
    // re-migration on the next boot).
    super::tx::run_tx(conn, "BEGIN", || async move {
        conn.execute("DELETE FROM schema_version", ()).await?;
        conn.execute(
            "INSERT INTO schema_version(version) VALUES (?1)",
            libsql::params![version],
        )
        .await?;
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::{current_version, set_version};
    use crate::libsql_store::LibsqlStore;

    /// Bug 11: `set_version` wraps DELETE + INSERT in a single transaction so
    /// the `schema_version` table is never left empty (nor duplicated) by a
    /// crash between the two statements. The replace-not-duplicate invariant
    /// is invisible to `current_version` (it uses `LIMIT 1`), so the row count
    /// is asserted directly.
    #[tokio::test]
    async fn set_version_replaces_single_row_atomically() {
        let store = LibsqlStore::open_memory().await.unwrap();
        let conn = store.conn().await.unwrap();

        // `bootstrap` already seeded exactly one row at SCHEMA_VERSION; each
        // subsequent set_version must replace it, never append a second row.
        set_version(&conn, 42).await.unwrap();
        assert_eq!(current_version(&conn).await.unwrap(), Some(42));

        set_version(&conn, 7).await.unwrap();
        assert_eq!(current_version(&conn).await.unwrap(), Some(7));

        // The core regression guard: exactly one row remains. A naive
        // double-INSERT (or a DELETE that outran a crashed INSERT) would leave
        // 2 (or 0) rows here.
        let stmt = conn
            .prepare("SELECT COUNT(*) FROM schema_version")
            .await
            .unwrap();
        let mut rows = stmt.query(()).await.unwrap();
        let count: i64 = rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap();
        assert_eq!(count, 1, "schema_version must hold exactly one row");
    }
}
