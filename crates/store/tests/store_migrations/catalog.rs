//! Message-display and team-ledger migrations.

use opencoder_store::{LibsqlStore, Store};
use tempfile::TempDir;

async fn fresh() -> (TempDir, LibsqlStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();
    (dir, store)
}

async fn index_count(store: &LibsqlStore) -> i64 {
    let conn = store.conn().await.unwrap();
    let stmt = conn
        .prepare(
            "SELECT COUNT(*) FROM sqlite_master \
             WHERE type = 'index' AND name = 'idx_subagent_task_id'",
        )
        .await
        .unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    rows.next()
        .await
        .unwrap()
        .expect("COUNT row")
        .get::<i64>(0)
        .unwrap()
}

/// Bootstrap creates (and re-creates idempotently) the `task_id` index on
/// `subagent_tasks`: the COMPLETE / CANCEL / get-by-task-id paths filter by
/// `task_id` alone and previously full-scanned the table on every replay /
/// interrupt probe.
#[tokio::test]
async fn bootstrap_creates_subagent_task_id_index() {
    let (dir, store) = fresh().await;
    assert_eq!(
        index_count(&store).await,
        1,
        "idx_subagent_task_id must exist after bootstrap"
    );

    // Reopening the same database (idempotent bootstrap) must not fail or
    // duplicate the index.
    drop(store);
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();
    assert_eq!(
        index_count(&store).await,
        1,
        "idx_subagent_task_id stays singular across reopens"
    );
}

/// v13 -> v14: `messages.display` (verbatim echo text) is added as a nullable
/// column; existing rows read back as `None` and new rows round-trip.
#[tokio::test]
async fn schema_migration_v13_to_v14_adds_message_display() {
    use libsql::Builder;
    use opencoder_core::Message;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("migrate-v14.db");

    // Phase 1: hand-write a v13 messages table (full v13 column set, NO
    // display) plus a matching sessions table, and stamp version 13.
    {
        let db = Builder::new_local(&db_path).build().await.unwrap();
        let conn = db.connect().unwrap();
        conn.execute("CREATE TABLE schema_version (version INTEGER NOT NULL)", ())
            .await
            .unwrap();
        conn.execute(
            "CREATE TABLE sessions (               id TEXT PRIMARY KEY, title TEXT, agent TEXT, model TEXT, workdir_hash TEXT,               created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,               summary TEXT, summary_seq INTEGER, summary_images_json TEXT,               handoff_seq INTEGER, handoff_plan TEXT, skill TEXT,               task_type TEXT NOT NULL DEFAULT 'parent', requirement TEXT,               plan_snapshot TEXT, plan_input_count INTEGER NOT NULL DEFAULT 0,               autopilot_mode TEXT)",
            (),
        )
        .await
        .unwrap();
        conn.execute(
            "CREATE TABLE messages (               seq INTEGER PRIMARY KEY AUTOINCREMENT,               id TEXT NOT NULL,               session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,               role TEXT NOT NULL, agent TEXT, model TEXT,               blocks_json TEXT NOT NULL, usage_json TEXT NOT NULL,               created_at INTEGER NOT NULL, synthetic INTEGER NOT NULL DEFAULT 0,               mode TEXT, summary INTEGER NOT NULL DEFAULT 0)",
            (),
        )
        .await
        .unwrap();
        conn.execute("INSERT INTO schema_version (version) VALUES (13)", ())
            .await
            .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES ('s13', 1, 1)",
            (),
        )
        .await
        .unwrap();
        conn.execute(
            "INSERT INTO messages (id, session_id, role, blocks_json, usage_json, created_at, synthetic)              VALUES ('m0', 's13', 'user', '[{\"kind\":\"text\",\"text\":\"old\"}]', '{}', 1, 0)",
            (),
        )
        .await
        .unwrap();
    }

    // Phase 2: reopen — migrate(conn, 13) runs the `if from < 14` block.
    let store = LibsqlStore::open(&db_path).await.unwrap();

    // (1) The display column now exists on messages.
    {
        let conn = store.conn().await.unwrap();
        let stmt = conn.prepare("PRAGMA table_info(messages)").await.unwrap();
        let mut rows = stmt.query(()).await.unwrap();
        let mut found = false;
        while let Some(row) = rows.next().await.unwrap() {
            let name: String = row.get(1).unwrap();
            if name == "display" {
                found = true;
                break;
            }
        }
        assert!(
            found,
            "messages.display column must exist after v13->v14 migration"
        );
    }

    // (2) The pre-existing row survives and reads back as display=None.
    let loaded = store.load_messages("s13").await.unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].text(), "old");
    assert!(
        loaded[0].display.is_none(),
        "v13 row: display reads as NULL after migration"
    );

    // (3) New messages round-trip the display column through the migrated DB.
    let mut msg = Message::user("m1", " fix it");
    msg.display = Some("$review fix it".into());
    store.append_messages("s13", &[msg]).await.unwrap();
    let loaded = store.load_messages("s13").await.unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[1].display.as_deref(), Some("$review fix it"));
}

/// v16 -> v17: the `team_topic_runs` ledger (opencoder-team fan-out) must
/// exist and be writable after upgrading a hand-written v16 database. The
/// nodes table is pre-created in its v16 shape (v12-era DDL, unchanged since)
/// so the ledger's FK parent exists exactly as a real v16 database would
/// have it, and register_node works against the pre-existing table.
#[tokio::test]
async fn schema_migration_v16_to_v17_adds_team_topic_runs() {
    use libsql::Builder;
    use opencoder_store::{TeamTopicRunRecord, TEAM_RUN_EXECUTING, TEAM_RUN_FINISHED};

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("migrate-v17.db");

    // Phase 1: hand-write a v16 database — nodes registry + version stamp 16.
    // team_topic_runs deliberately absent: that is what v17 adds.
    {
        let db = Builder::new_local(&db_path).build().await.unwrap();
        let conn = db.connect().unwrap();
        conn.execute("CREATE TABLE schema_version (version INTEGER NOT NULL)", ())
            .await
            .unwrap();
        conn.execute(
            "CREATE TABLE nodes (               id            TEXT PRIMARY KEY,               name          TEXT NOT NULL UNIQUE,               version       TEXT,               workdir       TEXT,               first_seen    INTEGER NOT NULL,               last_seen_at  INTEGER NOT NULL,               last_status   TEXT NOT NULL DEFAULT 'online',               last_task_id  TEXT,               last_addr     TEXT)",
            (),
        )
        .await
        .unwrap();
        conn.execute("INSERT INTO schema_version (version) VALUES (16)", ())
            .await
            .unwrap();
    }

    // Phase 2: reopen — bootstrap's CREATE batch stamps the missing table,
    // migrate(conn, 16) runs the `if from < 17` block, version lands at 17.
    let store = LibsqlStore::open(&db_path).await.unwrap();

    // (1) The version bumped to 17.
    {
        let conn = store.conn().await.unwrap();
        let stmt = conn
            .prepare("SELECT version FROM schema_version LIMIT 1")
            .await
            .unwrap();
        let mut rows = stmt.query(()).await.unwrap();
        let r = rows.next().await.unwrap().expect("version row exists");
        let v: i64 = r.get(0).unwrap();
        assert_eq!(v, 25, "schema version must be latest after v16 migration");
    }

    // (2) The table exists (write proves it) and the pre-existing nodes
    // table is reused — register lands in the hand-written registry.
    let node = store
        .register_node("node-v16", Some("v1"), None, None, 1_000)
        .await
        .unwrap();
    assert_eq!(node.name, "node-v16");
    assert_eq!(node.first_seen, 1_000);

    store
        .upsert_team_topic_run(&TeamTopicRunRecord {
            topic_id: "topic-mig".into(),
            node_id: node.id.clone(),
            status: TEAM_RUN_EXECUTING.into(),
            created_at: 1_234,
        })
        .await
        .unwrap();
    store.finish_team_topic_run("topic-mig").await.unwrap();
    let runs = store.list_team_topic_runs("topic-mig").await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, TEAM_RUN_FINISHED);
    assert_eq!(runs[0].created_at, 1_234);

    // (3) The topic index landed too (post-migrate batch).
    {
        let conn = store.conn().await.unwrap();
        let stmt = conn
            .prepare(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_team_topic_runs_topic'",
            )
            .await
            .unwrap();
        let mut rows = stmt.query(()).await.unwrap();
        let r = rows.next().await.unwrap().unwrap();
        let n: i64 = r.get(0).unwrap();
        assert_eq!(n, 1, "idx_team_topic_runs_topic must exist after migration");
    }
}

/// v20 adds the project todo executor dimension: `project_todos` gains
/// executor_kind/ref/spec, `project_todo_runs` gains executor_kind plus the
/// brain-provenance and artifact/topic refs. Legacy rows must backfill to the
/// agent flow (`NOT NULL DEFAULT 'agent'`) and new rows must round-trip.
#[tokio::test]
async fn schema_migration_v19_to_v20_adds_project_executor_columns() {
    use libsql::Builder;
    use opencoder_store::{
        ProjectExecutorKind, ProjectStore, ProjectTodoRecord, ProjectTodoRunKind,
        ProjectTodoRunRecord, ProjectTodoRunStatus, ProjectTodoStatus,
    };

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("migrate-v20.db");

    // Phase 1: hand-write a v19 database — project tables in their pre-v20
    // shape (no executor columns) plus one legacy todo/run row each.
    {
        let db = Builder::new_local(&db_path).build().await.unwrap();
        let conn = db.connect().unwrap();
        conn.execute("CREATE TABLE schema_version (version INTEGER NOT NULL)", ())
            .await
            .unwrap();
        conn.execute(
            "CREATE TABLE project_todos (               id TEXT PRIMARY KEY,               milestone_id TEXT,               title TEXT NOT NULL,               draft TEXT NOT NULL,               plan_md TEXT,               status TEXT NOT NULL,               agent TEXT NOT NULL,               active_session_id TEXT,               created_at INTEGER NOT NULL,               updated_at INTEGER NOT NULL)",
            (),
        )
        .await
        .unwrap();
        conn.execute(
            "CREATE TABLE project_todo_runs (               id TEXT PRIMARY KEY,               todo_id TEXT NOT NULL,               kind TEXT NOT NULL,               version INTEGER NOT NULL,               plan_md TEXT,               output_md TEXT,               agent TEXT NOT NULL,               session_id TEXT,               status TEXT NOT NULL,               started_at INTEGER NOT NULL,               finished_at INTEGER,               created_at INTEGER NOT NULL)",
            (),
        )
        .await
        .unwrap();
        conn.execute(
            "INSERT INTO project_todos (id, milestone_id, title, draft, plan_md, status, agent, active_session_id, created_at, updated_at) \
             VALUES ('legacy', NULL, 'legacy todo', 'draft', NULL, 'planned', 'act', NULL, 1, 1)",
            (),
        )
        .await
        .unwrap();
        conn.execute(
            "INSERT INTO project_todo_runs (id, todo_id, kind, version, plan_md, output_md, agent, session_id, status, started_at, finished_at, created_at) \
             VALUES ('legacy-run', 'legacy', 'execute', 1, NULL, NULL, 'act', NULL, 'done', 1, 2, 1)",
            (),
        )
        .await
        .unwrap();
        conn.execute("INSERT INTO schema_version (version) VALUES (19)", ())
            .await
            .unwrap();
    }

    // Phase 2: reopen — migrate(conn, 19) runs the `if from < 20` block and
    // stamps version 20.
    let store = LibsqlStore::open(&db_path).await.unwrap();
    {
        let conn = store.conn().await.unwrap();
        let stmt = conn
            .prepare("SELECT version FROM schema_version LIMIT 1")
            .await
            .unwrap();
        let mut rows = stmt.query(()).await.unwrap();
        let v: i64 = rows
            .next()
            .await
            .unwrap()
            .expect("version row exists")
            .get(0)
            .unwrap();
        assert_eq!(v, 25, "schema version must be latest after v19 migration");
    }

    // Legacy rows backfill to the agent flow.
    let legacy = store.get_todo("legacy").await.unwrap().unwrap();
    assert_eq!(legacy.status, ProjectTodoStatus::Planned);
    assert_eq!(legacy.executor_kind, ProjectExecutorKind::Agent);
    assert_eq!(legacy.executor_ref, None);
    assert_eq!(legacy.executor_spec, None);
    let legacy_run = store.get_todo_run("legacy-run").await.unwrap().unwrap();
    assert_eq!(legacy_run.executor_kind, ProjectExecutorKind::Agent);
    assert_eq!(legacy_run.capability_id, None);
    assert_eq!(legacy_run.plan_id, None);
    assert_eq!(legacy_run.output_ref, None);

    // New writes exercise every added column (todo: team ref + inline spec;
    // run: dag kind + brain provenance + artifact root).
    store
        .create_todo(&ProjectTodoRecord {
            id: "t20".into(),
            milestone_id: None,
            title: "v20 todo".into(),
            draft: "draft".into(),
            plan_md: None,
            status: ProjectTodoStatus::Planned,
            agent: "act".into(),
            executor_kind: ProjectExecutorKind::Team,
            executor_ref: Some("feature-team".into()),
            executor_spec: Some("{\"members\":[]}".into()),
            active_session_id: None,
            created_at: 5,
            updated_at: 5,
        })
        .await
        .unwrap();
    let t20 = store.get_todo("t20").await.unwrap().unwrap();
    assert_eq!(t20.executor_kind, ProjectExecutorKind::Team);
    assert_eq!(t20.executor_ref.as_deref(), Some("feature-team"));
    assert_eq!(t20.executor_spec.as_deref(), Some("{\"members\":[]}"));

    let run20 = ProjectTodoRunRecord {
        input_snapshot: None,
        trace_manifest: None,
        id: "run20".into(),
        todo_id: "t20".into(),
        kind: ProjectTodoRunKind::Execute,
        version: 1,
        plan_md: None,
        output_md: None,
        agent: "act".into(),
        executor_kind: ProjectExecutorKind::Dag,
        capability_id: Some("cap-1".into()),
        plan_id: Some("plan-1".into()),
        output_ref: Some("/workflow/w1/step-1/".into()),
        session_id: None,
        status: ProjectTodoRunStatus::Running,
        started_at: 6,
        finished_at: None,
        created_at: 6,
    };
    assert!(store.claim_todo_running_with_run(&run20, 6).await.unwrap());
    let claimed = store.get_todo_run("run20").await.unwrap().unwrap();
    assert_eq!(claimed.executor_kind, ProjectExecutorKind::Dag);
    assert_eq!(claimed.capability_id.as_deref(), Some("cap-1"));
    assert_eq!(claimed.plan_id.as_deref(), Some("plan-1"));
    assert_eq!(claimed.output_ref.as_deref(), Some("/workflow/w1/step-1/"));
}
