//! Migration from older legacy schema generations: real fleets carry stores
//! written before late columns (`kind`, `harness_runtime`, `requirement`,
//! `plan_snapshot`, `plan_input_count`, `autopilot_mode`,
//! `provider_state_json`, `display`, `sse_kind`, `promoted_seq`) and before
//! the `subagent_tasks` table existed. Extraction must project those as
//! `NULL` / zero rows instead of failing on `no such column`.

use std::path::Path;

use anyhow::Result;
use opencoder_database::Database;
use opencoder_local::legacy::{self, marker_path, read_marker, MarkerState, MigrateOptions};
use opencoder_store::{Delivery, RdbStore, Store};

const ROOT: &str = "sess-old";

/// Options for a fixture dir: source `<dir>/opencoder.db`, destination
/// `<dir>/opencoder.db.rdb`, default backup name.
fn opts_for(dir: &Path) -> MigrateOptions {
    MigrateOptions {
        source: dir.join("opencoder.db"),
        destination: dir.join("opencoder.db.rdb"),
        backup: legacy::default_backup(&dir.join("opencoder.db")),
        dry_run: false,
    }
}

/// One generation older than the final legacy schema: late columns dropped
/// everywhere, `summary_images_json` physically after `task_type`, and no
/// `subagent_tasks` table at all.
const DDL: &str = "\
CREATE TABLE IF NOT EXISTS sessions (
  id TEXT PRIMARY KEY, title TEXT, agent TEXT, model TEXT, workdir_hash TEXT,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, summary TEXT,
  summary_seq INTEGER, handoff_seq INTEGER, handoff_plan TEXT, skill TEXT,
  task_type TEXT NOT NULL DEFAULT 'parent', summary_images_json TEXT);
CREATE TABLE IF NOT EXISTS messages (
  seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL,
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  role TEXT NOT NULL, agent TEXT, model TEXT, blocks_json TEXT NOT NULL,
  usage_json TEXT NOT NULL, created_at INTEGER NOT NULL,
  synthetic INTEGER NOT NULL DEFAULT 0, mode TEXT,
  summary INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS session_events (
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  type TEXT NOT NULL, payload_json TEXT NOT NULL, ts INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS session_inputs (
  seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL,
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  delivery TEXT NOT NULL, prompt TEXT NOT NULL,
  images_json TEXT NOT NULL DEFAULT '[]', display_text TEXT,
  admitted_seq INTEGER NOT NULL, recorded INTEGER NOT NULL DEFAULT 0);
";

const ROWS: &str = "\
INSERT INTO sessions (id, title, agent, model, workdir_hash, created_at, updated_at, \
summary, summary_seq, handoff_seq, handoff_plan, skill, task_type) VALUES \
('sess-old', 'Old generation root', 'build', 'model-o', 'cafe01', 1000, 1600, \
NULL, NULL, NULL, NULL, NULL, 'parent');\n\
INSERT INTO messages (id, session_id, role, agent, model, blocks_json, usage_json, \
created_at, synthetic, mode, summary) VALUES \
('m-o1', 'sess-old', 'user', NULL, NULL, '[{\"kind\":\"text\",\"text\":\"old hello\"}]', \
'{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}', 1100, 0, NULL, 0), \
('m-o2', 'sess-old', 'assistant', 'build', 'model-o', \
'[{\"kind\":\"text\",\"text\":\"old world\"}]', \
'{\"input_tokens\":2,\"output_tokens\":2,\"total_tokens\":4}', 1500, 0, NULL, 0);\n\
INSERT INTO session_events (session_id, type, payload_json, ts) VALUES \
('sess-old', 'done', '{\"ok\":true}', 1600);\n\
INSERT INTO session_inputs (id, session_id, delivery, prompt, images_json, \
display_text, admitted_seq, recorded) VALUES \
('in-o1', 'sess-old', 'queue', 'old queued follow up', '[]', \
'old queued follow up', 1, 0);\n";

async fn seed(path: &Path) -> Result<()> {
    let database = Database::open(path).await?;
    database
        .write(move |conn| async move {
            conn.execute_batch(&format!("{DDL}{ROWS}")).await?;
            Ok(())
        })
        .await
}

#[tokio::test]
async fn migrates_older_schema_generation_with_null_projection() {
    let dir = tempfile::tempdir().unwrap();
    let opts = opts_for(dir.path());
    seed(&opts.source).await.unwrap();

    let report = legacy::run(&opts).await.unwrap();
    assert!(!report.already_migrated);
    assert_eq!(
        report.counts,
        legacy::MigrationCounts {
            roots: 1,
            sessions: 1,
            messages: 2,
            events: 1,
            inputs: 1,
            subagent_tasks: 0,
        }
    );
    assert_eq!(report.roots_imported, 1);
    assert!(report.backup_created);

    let marker = match read_marker(&marker_path(&opts.source)).unwrap() {
        MarkerState::Valid(marker) => *marker,
        state => panic!("expected valid marker, got {state:?}"),
    };
    assert_eq!(marker.counts.subagent_tasks, 0);

    // Destination content through the public store API: late columns read
    // back as their `None` forms, rows and pending inputs survive.
    let store = RdbStore::open(&opts.destination).await.unwrap();
    let session = store.get_session(ROOT).await.unwrap().expect("root");
    assert_eq!(session.title.as_deref(), Some("Old generation root"));
    assert_eq!(session.workdir_hash.as_deref(), Some("cafe01"));
    let messages = store.load_messages(ROOT).await.unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].text(), "old hello");
    assert_eq!(messages[1].text(), "old world");
    assert_eq!(store.events_after(ROOT, 0).await.unwrap().len(), 1);
    store.recover_orphan_inputs(ROOT).await.unwrap();
    assert_eq!(
        store
            .pending_inputs(ROOT, Delivery::Queue)
            .await
            .unwrap()
            .len(),
        1
    );

    // Marker idempotency holds for old-generation sources too.
    let again = legacy::run(&opts).await.unwrap();
    assert!(again.already_migrated);
}
