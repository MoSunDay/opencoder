// Scheduler tables bootstrap additively. Project v33 removes result caches;
// the release data-format gate excludes older Servers before this migration.
pub(crate) const SCHEMA_VERSION: i64 = 39;

// FULL durability is supported by Turso; NORMAL is not.
const PRAGMAS: &[&str] = &[
    "PRAGMA busy_timeout=30000",
    "PRAGMA synchronous=FULL",
    "PRAGMA journal_mode=WAL",
    "PRAGMA foreign_keys=ON",
    "PRAGMA cache_size=-65536",
];

const CREATE_SCHEMA_VERSION: &str =
    "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL)";
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
const CREATE_INDEX_IN_ID: &str =
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_inputs_session_id ON session_inputs(session_id, id)";
const CREATE_INDEX_EV: &str =
    "CREATE INDEX IF NOT EXISTS idx_events_session ON session_events(session_id, seq)";
const CREATE_INDEX_SA_PARENT: &str =
    "CREATE INDEX IF NOT EXISTS idx_subagent_parent ON subagent_tasks(parent_session_id, seq)";
const CREATE_INDEX_SA_CHILD: &str =
    "CREATE INDEX IF NOT EXISTS idx_subagent_child ON subagent_tasks(child_session_id)";
/// task_id lookups (COMPLETE / CANCEL / get-by-task-id) previously full-scanned
/// the table; replay and interrupt paths hit these per task.
const CREATE_INDEX_SA_TASK_ID: &str =
    "CREATE INDEX IF NOT EXISTS idx_subagent_task_id ON subagent_tasks(task_id)";
const CREATE_INDEX_SESSION_TASK_TYPE: &str =
    "CREATE INDEX IF NOT EXISTS idx_sessions_task_type ON sessions(task_type)";
const CREATE_INDEX_SESSION_ACTIVITY: &str =
    "CREATE INDEX IF NOT EXISTS idx_sessions_activity ON sessions(MAX(updated_at, created_at) DESC, id DESC)";
const CREATE_INDEX_TODO_STATUS: &str =
    "CREATE INDEX IF NOT EXISTS idx_todo_workflows_status ON todo_workflows(status, updated_at)";
const CREATE_INDEX_TODO_EVENTS: &str =
    "CREATE INDEX IF NOT EXISTS idx_todo_events_workflow ON todo_events(workflow_id, seq)";
const CREATE_NODES: &str = "\
CREATE TABLE IF NOT EXISTS nodes (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL UNIQUE,
  version       TEXT,
  workdir       TEXT,
  first_seen    INTEGER NOT NULL,
  last_seen_at  INTEGER NOT NULL,
  last_status   TEXT NOT NULL DEFAULT 'online',
  last_task_id  TEXT,
  last_addr     TEXT
)";
const CREATE_NODE_TASKS: &str = "\
CREATE TABLE IF NOT EXISTS node_tasks (
  id              TEXT PRIMARY KEY,
  node_id         TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
  session_id      TEXT NOT NULL UNIQUE REFERENCES sessions(id) ON DELETE CASCADE,
  title           TEXT,
  prompt          TEXT NOT NULL,
  agent           TEXT,
  model           TEXT,
  status          TEXT NOT NULL DEFAULT 'pending',
  error           TEXT,
  cancel_requested INTEGER NOT NULL DEFAULT 0,
  created_at      INTEGER NOT NULL,
  claimed_at      INTEGER,
  finished_at     INTEGER
)";
/// Node claim polling filters by `(node_id, status)` (single-active-task guard
/// plus the oldest-pending FIFO scan), so the index covers both branches.
const CREATE_INDEX_NODE_TASKS_STATUS: &str =
    "CREATE INDEX IF NOT EXISTS idx_node_tasks_node_status ON node_tasks(node_id, status)";
/// Project-module tables (v15). No FK constraints on purpose: the project
/// store cascades deletes explicitly (see `turso_store/project.rs`) so
/// every backend behaves the same.
const CREATE_PROJECT_GOALS: &str = "\
CREATE TABLE IF NOT EXISTS project_goals (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  detail_md TEXT,
  status TEXT NOT NULL,
  sort_key INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  current_summary TEXT NOT NULL DEFAULT '',
  summary_updated_at INTEGER,
  updated_at INTEGER NOT NULL
)";
const CREATE_PROJECT_INITIATIVES: &str = "\
CREATE TABLE IF NOT EXISTS project_initiatives (
  id TEXT PRIMARY KEY,
  goal_id TEXT,
  title TEXT NOT NULL,
  detail_md TEXT,
  sort_key INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  current_summary TEXT NOT NULL DEFAULT '',
  summary_updated_at INTEGER,
  updated_at INTEGER NOT NULL
)";
const CREATE_PROJECT_TODOS: &str = "\
CREATE TABLE IF NOT EXISTS project_todos (
  id TEXT PRIMARY KEY,
  initiative_id TEXT,
  title TEXT NOT NULL,
  draft TEXT NOT NULL,
  plan_md TEXT,
  status TEXT NOT NULL,
  agent TEXT NOT NULL,
  active_session_id TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  executor_kind TEXT NOT NULL DEFAULT 'agent',
  executor_ref TEXT,
  executor_spec TEXT,
  board_status TEXT NOT NULL DEFAULT 'backlog',
  position INTEGER NOT NULL DEFAULT 0,
  capability_id TEXT
)";
const CREATE_PROJECT_TODO_RUNS: &str = "\
CREATE TABLE IF NOT EXISTS project_todo_runs (
  id TEXT PRIMARY KEY,
  todo_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  version INTEGER NOT NULL,
  plan_md TEXT,
  output_md TEXT,
  agent TEXT NOT NULL,
  session_id TEXT,
  status TEXT NOT NULL,
  started_at INTEGER NOT NULL,
  finished_at INTEGER,
  created_at INTEGER NOT NULL,
  executor_kind TEXT NOT NULL DEFAULT 'agent',
  capability_id TEXT,
  plan_id TEXT,
  output_ref TEXT,
  input_snapshot TEXT,
  trace_manifest TEXT
)";
const CREATE_PROJECT_TODO_EXECUTIONS: &str = "\
CREATE TABLE IF NOT EXISTS project_todo_executions (
  todo_id TEXT NOT NULL,
  execution_id TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  kind TEXT NOT NULL DEFAULT '',
  name TEXT NOT NULL DEFAULT '',
  capability_id TEXT,
  PRIMARY KEY (todo_id, execution_id)
)";
const CREATE_INDEX_PROJECT_INITIATIVES_GOAL: &str =
    "CREATE INDEX IF NOT EXISTS idx_project_initiatives_goal ON project_initiatives(goal_id)";
const CREATE_INDEX_PROJECT_TODOS_INITIATIVE: &str =
    "CREATE INDEX IF NOT EXISTS idx_project_todos_initiative ON project_todos(initiative_id)";
const CREATE_INDEX_PROJECT_TODO_RUNS_TODO: &str =
    "CREATE INDEX IF NOT EXISTS idx_project_todo_runs_todo ON project_todo_runs(todo_id)";
const CREATE_BRAIN_CAPABILITIES: &str = "\
CREATE TABLE IF NOT EXISTS brain_capabilities (
  id TEXT PRIMARY KEY,
  capability_type TEXT NOT NULL,
  summary TEXT NOT NULL,
  input_desc TEXT NOT NULL,
  output_desc TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
)";
const CREATE_BRAIN_ENG_INPUTS: &str = "\
CREATE TABLE IF NOT EXISTS brain_eng_inputs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  capability_id TEXT NOT NULL REFERENCES brain_capabilities(id) ON DELETE CASCADE,
  content TEXT NOT NULL,
  position INTEGER NOT NULL
)";
/// Embeddings for the brain capability library. `emb` holds the vector as
/// little-endian f32 bytes — the encoding turso's bundled `vector32()`
/// accepts directly (blob binding, verified by the brain_store integration
/// test), so `vector_distance_cos(v.emb, vector32(?))` needs no conversion.
/// `capability_id` is the PK: one (latest) embedding per capability.
const CREATE_BRAIN_VECTORS: &str = "\
CREATE TABLE IF NOT EXISTS brain_vectors (
  capability_id TEXT PRIMARY KEY REFERENCES brain_capabilities(id) ON DELETE CASCADE,
  dim INTEGER NOT NULL,
  model TEXT NOT NULL,
  emb BLOB NOT NULL,
  updated_at INTEGER NOT NULL
)";
/// Platform users (v24): bearer identities beyond the seed admin token.
/// Only sha256 hex digests are stored; the plaintext token is shown exactly
/// once in the creation response and never persisted or logged.
const CREATE_PLATFORM_USERS: &str = access::USERS;

/// Exemplar-input lookups are per-capability ordered scans; the index keeps
/// the brain catalog's `get`/`list` reads off full table scans.
const CREATE_INDEX_BRAIN_ENG_INPUTS: &str =
    "CREATE INDEX IF NOT EXISTS idx_brain_eng_inputs_cap ON brain_eng_inputs(capability_id)";
/// DAG workflow tables (v16): definitions, dispatched runs, node-uploaded
/// events. No FK constraints on purpose (same policy as the project tables):
/// `dag_runs.spec_json` is a dispatch-time snapshot, so runs intentionally
/// outlive def edits, and a deleted node leaves its runs to the lost-node
/// sweep instead of a cascade.
const CREATE_DAG_DEFS: &str = "\
CREATE TABLE IF NOT EXISTS dag_defs (
  id         TEXT PRIMARY KEY,
  name       TEXT NOT NULL UNIQUE,
  spec_json  TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
)";
const CREATE_DAG_RUNS: &str = "\
CREATE TABLE IF NOT EXISTS dag_runs (
  id          TEXT PRIMARY KEY,
  dag_id      TEXT NOT NULL,
  name        TEXT NOT NULL,
  spec_json   TEXT NOT NULL,
  node_id     TEXT,
  status      TEXT NOT NULL,
  error       TEXT,
  created_at  INTEGER NOT NULL,
  claimed_at  INTEGER,
  finished_at INTEGER
)";
const CREATE_DAG_EVENTS: &str = "\
CREATE TABLE IF NOT EXISTS dag_events (
  seq     INTEGER PRIMARY KEY AUTOINCREMENT,
  run_id  TEXT NOT NULL,
  kind    TEXT NOT NULL,
  step    TEXT,
  payload TEXT NOT NULL,
  at_ms   INTEGER NOT NULL
)";
/// DAG claim polling filters by `(status, node_id)`: the oldest-pending FIFO
/// scan (`status='pending' AND (node_id IS NULL OR node_id=?)`) plus the
/// single-active-run guard (`node_id=? AND status='running'`), so the index
/// covers both branches (same rationale as the node_tasks index).
const CREATE_INDEX_DAG_RUNS_STATUS_NODE: &str =
    "CREATE INDEX IF NOT EXISTS idx_dag_runs_status_node ON dag_runs(status, node_id)";
/// Run-SSE replay scans `run_id` with a `seq > after` cursor, ascending.
const CREATE_INDEX_DAG_EVENTS_RUN_SEQ: &str =
    "CREATE INDEX IF NOT EXISTS idx_dag_events_run_seq ON dag_events(run_id, seq)";

/// Apply WAL + safety pragmas to a single connection. Cheap to call per-acquire.
///
/// Uses `query` (not `execute`) because some pragmas (e.g. `journal_mode=WAL`)
/// return rows. Drain each result before applying the next pragma.
pub async fn apply_connection_pragmas(conn: &Connection) -> Result<()> {
    for p in PRAGMAS {
        let mut stmt = conn
            .prepare_cached(p)
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
    let mut stmt = conn
        .prepare_cached("PRAGMA wal_checkpoint(PASSIVE)")
        .await?;
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
///
/// The whole bootstrap runs inside ONE `BEGIN IMMEDIATE` transaction: the 17
/// DDL statements would otherwise each auto-commit (17 write-amplifying
/// commits per open). `BEGIN IMMEDIATE` plus the already-active
/// `busy_timeout` makes concurrent openers queue on the single write lock
/// instead of failing, and any failing step rolls the entire bootstrap (in
/// particular the migration sequence) back atomically.
///
/// Databases whose tables predate schema_version tracking (tables present,
/// version row absent) are detected by probing `sqlite_master` BEFORE the DDL
/// batch and are upgraded through a full `migrate(0)` pass instead of being
/// stamped with the current version untouched: stamping alone would leave
/// `sessions.task_type` missing, fail the post-migration index batch, and -
/// batch and stamp sharing one transaction - roll the whole bootstrap back so
/// the database could never be opened.
pub async fn bootstrap(conn: &Connection) -> Result<()> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || bootstrap_tx(conn)).await
}
