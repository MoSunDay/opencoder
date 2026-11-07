//! Fixture builder for the legacy-migration integration tests: recreates a
//! real legacy SQLite session store (schema exactly as the last SQLite build
//! shipped it, including columns the rdb era dropped) through the database
//! crate's pooled writer, so the migration under test reads a real file.

use std::path::Path;

use anyhow::Result;
use opencoder_database::Database;

/// Session ids shared between the fixture and the test assertions.
pub const ALPHA: &str = "sess-alpha";
pub const BETA: &str = "sess-beta";
pub const GAMMA: &str = "sess-gamma";

/// Default source/backup/marker names inside a fixture data dir.
pub const SOURCE_NAME: &str = "opencoder.db";

/// Two roots (`ALPHA` created first, `BETA` second) plus `GAMMA`, a subagent
/// child of `ALPHA`. Expected migrated totals: 2 roots, 3 sessions, 4
/// messages (the `BETA` compaction row with `summary = 1` is excluded), 4
/// events, 2 pending inputs, 1 subagent task.
pub fn expected_counts() -> opencoder_local::legacy::MigrationCounts {
    opencoder_local::legacy::MigrationCounts {
        roots: 2,
        sessions: 3,
        messages: 4,
        events: 4,
        inputs: 2,
        subagent_tasks: 1,
    }
}

/// Legacy DDL with foreign keys, as the legacy build declared them.
const DDL: &str = "\
CREATE TABLE IF NOT EXISTS sessions (
  id TEXT PRIMARY KEY, title TEXT, agent TEXT, model TEXT, workdir_hash TEXT,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, summary TEXT,
  summary_seq INTEGER, summary_images_json TEXT, handoff_seq INTEGER,
  handoff_plan TEXT, skill TEXT, task_type TEXT NOT NULL DEFAULT 'parent',
  requirement TEXT, plan_snapshot TEXT, plan_input_count INTEGER NOT NULL DEFAULT 0,
  autopilot_mode TEXT, harness_runtime TEXT, kind TEXT);
CREATE TABLE IF NOT EXISTS messages (
  seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL,
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  role TEXT NOT NULL, agent TEXT, model TEXT, blocks_json TEXT NOT NULL,
  provider_state_json TEXT, usage_json TEXT NOT NULL, created_at INTEGER NOT NULL,
  synthetic INTEGER NOT NULL DEFAULT 0, display TEXT, mode TEXT,
  summary INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS session_inputs (
  seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL,
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  delivery TEXT NOT NULL, prompt TEXT NOT NULL,
  images_json TEXT NOT NULL DEFAULT '[]', display_text TEXT,
  admitted_seq INTEGER NOT NULL, promoted_seq INTEGER,
  recorded INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS session_events (
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  type TEXT NOT NULL, payload_json TEXT NOT NULL, sse_kind TEXT, ts INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS subagent_tasks (
  seq INTEGER PRIMARY KEY AUTOINCREMENT, task_id TEXT NOT NULL,
  parent_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  child_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  parent_message_id TEXT, agent TEXT NOT NULL, prompt TEXT NOT NULL, result TEXT,
  status TEXT NOT NULL, ok INTEGER, started_at INTEGER NOT NULL, completed_at INTEGER);
";

/// Same tables without foreign keys, for fixtures that deliberately hold a
/// dangling `subagent_tasks` reference (a corrupt store the migration must
/// reject instead of silently dropping).
const DDL_NO_FK: &str = "\
CREATE TABLE IF NOT EXISTS sessions (
  id TEXT PRIMARY KEY, title TEXT, agent TEXT, model TEXT, workdir_hash TEXT,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, summary TEXT,
  summary_seq INTEGER, summary_images_json TEXT, handoff_seq INTEGER,
  handoff_plan TEXT, skill TEXT, task_type TEXT NOT NULL DEFAULT 'parent',
  requirement TEXT, plan_snapshot TEXT, plan_input_count INTEGER NOT NULL DEFAULT 0,
  autopilot_mode TEXT, harness_runtime TEXT, kind TEXT);
CREATE TABLE IF NOT EXISTS messages (
  seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL, session_id TEXT NOT NULL,
  role TEXT NOT NULL, agent TEXT, model TEXT, blocks_json TEXT NOT NULL,
  provider_state_json TEXT, usage_json TEXT NOT NULL, created_at INTEGER NOT NULL,
  synthetic INTEGER NOT NULL DEFAULT 0, display TEXT, mode TEXT,
  summary INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS session_inputs (
  seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL, session_id TEXT NOT NULL,
  delivery TEXT NOT NULL, prompt TEXT NOT NULL,
  images_json TEXT NOT NULL DEFAULT '[]', display_text TEXT,
  admitted_seq INTEGER NOT NULL, promoted_seq INTEGER, recorded INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS session_events (
  seq INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL,
  type TEXT NOT NULL, payload_json TEXT NOT NULL, sse_kind TEXT, ts INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS subagent_tasks (
  seq INTEGER PRIMARY KEY AUTOINCREMENT, task_id TEXT NOT NULL,
  parent_session_id TEXT NOT NULL, child_session_id TEXT NOT NULL,
  parent_message_id TEXT, agent TEXT NOT NULL, prompt TEXT NOT NULL, result TEXT,
  status TEXT NOT NULL, ok INTEGER, started_at INTEGER NOT NULL, completed_at INTEGER);
";

/// All fixture rows. Obsolete columns (`plan_snapshot`, `plan_input_count`,
/// `harness_runtime`, `recorded`) carry values that must be ignored.
const ROWS: &str = "\
INSERT INTO sessions (id, kind, title, agent, model, autopilot_mode, workdir_hash, \
created_at, updated_at, summary, summary_seq, summary_images_json, handoff_seq, \
handoff_plan, skill, task_type, requirement, plan_snapshot, plan_input_count, \
harness_runtime) VALUES \
('sess-alpha', NULL, 'Alpha root', 'build', 'model-a', NULL, 'deadbeef01', 1000, 1500, \
NULL, NULL, NULL, NULL, NULL, NULL, 'parent', NULL, 'plan-stale', 7, 'legacy-harness'), \
('sess-gamma', 'agent', 'Gamma child', 'research', 'model-a', NULL, 'deadbeef01', 1200, \
1300, NULL, NULL, '[\"https://example.com/g.png\"]', NULL, NULL, NULL, 'subagent', \
'research the thing', 'plan-child', 1, 'legacy-harness'), \
('sess-beta', NULL, 'Beta root', 'build', 'model-b', 'ap', NULL, 2000, 2100, \
'kept summary', 5, NULL, 2, 'handoff-plan', 'skill-x', 'parent', NULL, NULL, 0, NULL);\n\
INSERT INTO messages (id, session_id, role, agent, model, blocks_json, \
provider_state_json, usage_json, created_at, synthetic, display, mode, summary) VALUES \
('m-a1', 'sess-alpha', 'user', NULL, NULL, '[{\"kind\":\"text\",\"text\":\"hello\"}]', \
NULL, '{\"input_tokens\":1,\"output_tokens\":2,\"total_tokens\":3}', 1100, 0, \
'hello', NULL, 0), \
('m-a2', 'sess-alpha', 'assistant', 'build', 'model-a', \
'[{\"kind\":\"text\",\"text\":\"world\"}]', NULL, \
'{\"input_tokens\":10,\"output_tokens\":20,\"total_tokens\":30}', 1200, 0, NULL, NULL, 0), \
('m-g1', 'sess-gamma', 'assistant', 'research', 'model-a', \
'[{\"kind\":\"text\",\"text\":\"child done\"}]', NULL, \
'{\"input_tokens\":4,\"output_tokens\":5,\"total_tokens\":9}', 1250, 0, NULL, NULL, 0), \
('m-b1', 'sess-beta', 'user', NULL, NULL, '[{\"kind\":\"text\",\"text\":\"beta prompt\"}]', \
NULL, '{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}', 2050, 0, \
'beta prompt', NULL, 0), \
('m-b2', 'sess-beta', 'assistant', 'build', 'model-b', \
'[{\"kind\":\"text\",\"text\":\"compaction artifact\"}]', NULL, \
'{\"input_tokens\":0,\"output_tokens\":0,\"total_tokens\":0}', 2060, 1, NULL, NULL, 1);\n\
INSERT INTO session_events (session_id, type, payload_json, sse_kind, ts) VALUES \
('sess-alpha', 'prompt_admitted', '{\"prompt\":\"hello\"}', NULL, 1100), \
('sess-alpha', 'done', '{\"ok\":true}', NULL, 1400), \
('sess-gamma', 'done', '{\"ok\":true}', NULL, 1250), \
('sess-beta', 'step', '{\"n\":1}', 'step', 2050);\n\
INSERT INTO session_inputs (id, session_id, delivery, prompt, images_json, \
display_text, admitted_seq, promoted_seq, recorded) VALUES \
('in-1', 'sess-alpha', 'steer', 'steer me now', '[]', 'steer me now', 1, NULL, 1), \
('in-2', 'sess-alpha', 'queue', 'queued follow up', '[\"https://example.com/q.png\"]', \
NULL, 2, NULL, 0);\n\
INSERT INTO subagent_tasks (task_id, parent_session_id, child_session_id, \
parent_message_id, agent, prompt, result, status, ok, started_at, completed_at) VALUES \
('task-1', 'sess-alpha', 'sess-gamma', 'm-a2', 'research', 'research the thing', \
'child finished', 'completed', 1, 1200, 1300);";

/// Seed the standard fixture (foreign keys enforced).
pub async fn seed(path: &Path) -> Result<()> {
    run_batch(path, format!("{DDL}{ROWS}")).await
}

/// Seed the standard fixture, then apply `extra` SQL in the same batch —
/// used to corrupt a cell after the consistent rows exist.
pub async fn seed_with(path: &Path, extra: &str) -> Result<()> {
    run_batch(path, format!("{DDL}{ROWS}\n{extra}")).await
}

/// Seed without foreign keys plus a `subagent_tasks` row pointing at a child
/// session that does not exist.
pub async fn seed_orphan_task(path: &Path) -> Result<()> {
    let ghost = "INSERT INTO subagent_tasks (task_id, parent_session_id, \
child_session_id, parent_message_id, agent, prompt, result, status, ok, \
started_at, completed_at) VALUES ('task-2', 'sess-beta', 'sess-ghost', NULL, \
'ghost', 'orphan', NULL, 'running', NULL, 2100, NULL);";
    run_batch(path, format!("{DDL_NO_FK}{ROWS}\n{ghost}")).await
}

async fn run_batch(path: &Path, sql: String) -> Result<()> {
    let database = Database::open(path).await?;
    database
        .write(move |conn| async move {
            conn.execute_batch(&sql).await?;
            Ok(())
        })
        .await
}
