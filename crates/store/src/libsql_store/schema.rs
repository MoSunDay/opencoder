use anyhow::{Context, Result};
use libsql::Connection;

const SCHEMA_VERSION: i64 = 3;

const PRAGMAS: &[&str] = &[
    "PRAGMA journal_mode=WAL",
    "PRAGMA synchronous=NORMAL",
    "PRAGMA busy_timeout=5000",
    "PRAGMA foreign_keys=ON",
    "PRAGMA cache_size=-65536",
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
  summary_seq  INTEGER,
  handoff_seq  INTEGER,
  handoff_plan TEXT,
  skill        TEXT
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
  admitted_seq INTEGER NOT NULL,
  promoted_seq INTEGER
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
const CREATE_INDEX_MSG: &str =
    "CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, seq)";
const CREATE_INDEX_IN: &str = "CREATE INDEX IF NOT EXISTS idx_inputs_pending ON session_inputs(session_id, promoted_seq, delivery, admitted_seq)";
const CREATE_INDEX_EV: &str =
    "CREATE INDEX IF NOT EXISTS idx_events_session ON session_events(session_id, seq)";
const CREATE_INDEX_SA_PARENT: &str =
    "CREATE INDEX IF NOT EXISTS idx_subagent_parent ON subagent_tasks(parent_session_id, seq)";
const CREATE_INDEX_SA_CHILD: &str =
    "CREATE INDEX IF NOT EXISTS idx_subagent_child ON subagent_tasks(child_session_id)";

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

/// Create all tables if absent, run incremental migrations, and record the
/// schema version. Idempotent: safe on fresh and existing databases.
pub async fn bootstrap(conn: &Connection) -> Result<()> {
    conn.execute(CREATE_SCHEMA_VERSION, ()).await?;
    conn.execute(CREATE_SESSIONS, ()).await?;
    conn.execute(CREATE_MESSAGES, ()).await?;
    conn.execute(CREATE_INPUTS, ()).await?;
    conn.execute(CREATE_EVENTS, ()).await?;
    conn.execute(CREATE_SUBAGENT_TASKS, ()).await?;
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
    Ok(())
}

/// Run incremental schema migrations from `from` up to the current version.
async fn migrate(conn: &Connection, from: i64) -> Result<()> {
    if from < 2 {
        // v2: add sse_kind column to session_events for lossless event-kind
        // replay. The column is nullable so existing rows stay valid.
        conn.execute("ALTER TABLE session_events ADD COLUMN sse_kind TEXT", ())
            .await
            .context("migrate v2: add sse_kind column")?;
    }
    if from < 3 {
        // v3: plan→act handoff boundary + active skill on sessions, so resume
        // can reconstruct the post-handoff focused transcript and the active
        // skill across restarts. All nullable so existing rows stay valid.
        conn.execute("ALTER TABLE sessions ADD COLUMN handoff_seq INTEGER", ())
            .await
            .context("migrate v3: add handoff_seq column")?;
        conn.execute("ALTER TABLE sessions ADD COLUMN handoff_plan TEXT", ())
            .await
            .context("migrate v3: add handoff_plan column")?;
        conn.execute("ALTER TABLE sessions ADD COLUMN skill TEXT", ())
            .await
            .context("migrate v3: add skill column")?;
    }
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
    conn.execute("DELETE FROM schema_version", ()).await?;
    conn.execute(
        "INSERT INTO schema_version(version) VALUES (?1)",
        libsql::params![version],
    )
    .await?;
    Ok(())
}
