//! DDL for the core chat tables — sessions, messages, inputs, events,
//! subagent_tasks. Extracted from `schema.rs` (which assembles every
//! feature's DDL into the bootstrap batch and migration chain) so that
//! this file's strings stay next to the feature modules' own DDL while
//! `schema.rs` keeps the assembly role. The strings are verbatim moves,
//! not edits: schema identity is unchanged.

pub(super) const CREATE_SESSIONS: &str = "\
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
  plan_input_count INTEGER NOT NULL DEFAULT 0,
  autopilot_mode TEXT
)";
pub(super) const CREATE_MESSAGES: &str = "\
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
  display     TEXT,
  mode        TEXT,
  summary     INTEGER NOT NULL DEFAULT 0
)";
pub(super) const CREATE_INPUTS: &str = "\
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
pub(super) const CREATE_EVENTS: &str = "\
CREATE TABLE IF NOT EXISTS session_events (
  seq          INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id   TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  type         TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  sse_kind     TEXT,
  ts           INTEGER NOT NULL
)";
pub(super) const CREATE_SUBAGENT_TASKS: &str = "\
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
