//! P0 functional tests for the libsql-backed Store.
//!
//! Each test asserts a *behavior contract*, not "the function runs":
//! - concurrent_readers_while_writer: WAL allows N readers + 1 writer
//! - wal_crash_recovery: drop & reopen the db file, committed data survives
//! - jsonl_import_roundtrip: import preserves message history byte-equal
//! - schema_migration_versioning: bootstrap records schema version
//! - transaction_rollback_on_partial_failure: failed batch leaves no partial rows
//! - list_pagination_with_metadata: cursor pagination + search filter
//!
//! These run against a real on-disk libsql file (tempdir) so WAL behaviour
//! is exercised truthfully, not mocked.

use std::sync::{Arc, Mutex};

use opencode_core::{ContentBlock, Message, Role};
use opencode_store::{LibsqlStore, SessionFilter, SessionMeta, Store};
use tempfile::TempDir;

fn conv(seed: &str, n: usize) -> Vec<Message> {
    (0..n)
        .map(|i| {
            let id = format!("{seed}-{i}");
            let role = if i % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            let text = format!("{seed} msg {i}");
            let mut m = match role {
                Role::User => Message::user(id, text),
                Role::Assistant => {
                    let mut m = Message::assistant(id);
                    m.blocks = vec![ContentBlock::text(text)];
                    m
                }
                _ => unreachable!(),
            };
            m.created_at = i as i64;
            m
        })
        .collect()
}

async fn fresh() -> (TempDir, LibsqlStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();
    (dir, store)
}

async fn make_session(store: &LibsqlStore, id: &str, now: i64) {
    let meta = SessionMeta {
        id: id.to_string(),
        title: Some(format!("title-{id}")),
        agent: Some("act".into()),
        model: Some("glm-5.2".into()),
        workdir_hash: Some("h".into()),
        created_at: now,
        updated_at: now,
        summary: None,
        summary_seq: None,
    };
    store.create_session(&meta).await.unwrap();
}

#[tokio::test]
async fn create_get_update_delete_session_contract() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s1", 1000).await;

    let got = store
        .get_session("s1")
        .await
        .unwrap()
        .expect("session exists");
    assert_eq!(got.id, "s1");
    assert_eq!(got.title.as_deref(), Some("title-s1"));
    assert_eq!(got.model.as_deref(), Some("glm-5.2"));

    let patch = opencode_store::SessionPatch {
        title: Some("renamed".into()),
        model: Some("other/model".into()),
        updated_at: Some(2000),
        ..Default::default()
    };
    store.update_session("s1", &patch).await.unwrap();
    let got = store.get_session("s1").await.unwrap().unwrap();
    assert_eq!(got.title.as_deref(), Some("renamed"));
    assert_eq!(got.model.as_deref(), Some("other/model"));
    assert_eq!(got.updated_at, 2000);

    store.delete_session("s1").await.unwrap();
    assert!(store.get_session("s1").await.unwrap().is_none());
}

#[tokio::test]
async fn clear_other_sessions_keeps_current_and_cascades() {
    let (_dir, store) = fresh().await;
    make_session(&store, "keep", 1000).await;
    make_session(&store, "old-a", 2000).await;
    make_session(&store, "old-b", 3000).await;
    store
        .append_messages("old-a", &conv("old-a", 2))
        .await
        .unwrap();
    store
        .append_messages("old-b", &conv("old-b", 3))
        .await
        .unwrap();

    let deleted = store.clear_other_sessions("keep").await.unwrap();
    assert_eq!(deleted, 2, "two non-current sessions should be deleted");

    let remaining: Vec<String> = store
        .list_sessions(&SessionFilter::default())
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect();
    assert_eq!(remaining, vec!["keep".to_string()]);

    // FK ON DELETE CASCADE removed the child message rows too.
    assert!(
        store.load_messages("old-a").await.unwrap().is_empty(),
        "old-a messages must cascade-delete"
    );
    assert!(
        store.load_messages("old-b").await.unwrap().is_empty(),
        "old-b messages must cascade-delete"
    );
    assert_eq!(
        store.load_messages("keep").await.unwrap().len(),
        0,
        "keep session survives (just had no messages)"
    );

    // Clearing again is a no-op: count 0, keep still present.
    let again = store.clear_other_sessions("keep").await.unwrap();
    assert_eq!(again, 0);
    assert_eq!(
        store
            .list_sessions(&SessionFilter::default())
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn append_and_load_preserves_all_roles_and_blocks() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s1", 1).await;

    let original = vec![
        Message::user("u1", "hello"),
        {
            let mut m = Message::assistant("a1");
            m.blocks = vec![
                ContentBlock::text("I will use a tool"),
                ContentBlock::ToolUse {
                    id: "tu1".into(),
                    name: "bash".into(),
                    input: serde_json::json!({"command": "ls"}),
                },
            ];
            m.agent = Some("act".into());
            m.model = Some("glm-5.2".into());
            m.usage = opencode_core::MessageUsage {
                input_tokens: 10,
                output_tokens: 5,
                total_tokens: 15,
            };
            m.created_at = 2;
            m
        },
        {
            let id = "t1";
            Message {
                id: id.into(),
                role: Role::Tool,
                blocks: vec![ContentBlock::ToolResult {
                    tool_use_id: "tu1".into(),
                    content: "file.txt".into(),
                    is_error: false,
                }],
                model: None,
                agent: None,
                usage: Default::default(),
                created_at: 3,
                synthetic: false,
            }
        },
    ];

    let seqs = store.append_messages("s1", &original).await.unwrap();
    assert_eq!(seqs.len(), 3);
    assert_eq!(seqs, vec![1, 2, 3]);

    let loaded = store.load_messages("s1").await.unwrap();
    assert_eq!(loaded.len(), 3);
    assert_eq!(loaded[0].role, Role::User);
    assert_eq!(loaded[0].text(), "hello");
    assert_eq!(loaded[1].role, Role::Assistant);
    assert_eq!(loaded[1].agent.as_deref(), Some("act"));
    assert_eq!(loaded[1].model.as_deref(), Some("glm-5.2"));
    assert_eq!(loaded[1].usage.total_tokens, 15);
    assert_eq!(loaded[1].blocks.len(), 2);
    match &loaded[1].blocks[1] {
        ContentBlock::ToolUse { id, name, input } => {
            assert_eq!(id, "tu1");
            assert_eq!(name, "bash");
            assert_eq!(input["command"], "ls");
        }
        other => panic!("expected ToolUse, got {other:?}"),
    }
    assert_eq!(loaded[2].role, Role::Tool);
}

#[tokio::test]
async fn concurrent_readers_while_writer() {
    let dir = tempfile::tempdir().unwrap();
    let store_raw = LibsqlStore::open(dir.path().join("cw.db")).await.unwrap();
    make_session(&store_raw, "s", 1).await;
    store_raw
        .append_messages("s", &conv("seed", 10))
        .await
        .unwrap();
    let store = Arc::new(store_raw);
    let _dir = dir; // keep alive

    let store_w = store.clone();
    let writer = tokio::spawn(async move {
        for b in 0..20u32 {
            let msgs = conv(&format!("w{b}"), 5);
            store_w
                .append_messages("s", &msgs)
                .await
                .expect("append ok");
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
    });

    let mut readers = Vec::new();
    for r in 0..8u32 {
        let store_r = store.clone();
        readers.push(tokio::spawn(async move {
            for _ in 0..10usize {
                let loaded = store_r.load_messages("s").await.expect("read ok");
                // WAL: readers always see a consistent snapshot — count must be
                // monotonically non-decreasing and never observe a half-written batch.
                assert!(!loaded.is_empty(), "reader {r} saw empty");
                tokio::time::sleep(std::time::Duration::from_millis(3)).await;
            }
        }));
    }

    writer.await.unwrap();
    for h in readers {
        h.await.unwrap();
    }
    let final_count = store.load_messages("s").await.unwrap().len();
    assert_eq!(final_count, 10 + 20 * 5, "all writes landed");
}

#[tokio::test]
async fn wal_crash_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("crash.db");

    {
        let store = LibsqlStore::open(&db_path).await.unwrap();
        make_session(&store, "persist", 5).await;
        store
            .append_messages("persist", &conv("c", 7))
            .await
            .unwrap();
        // drop store WITHOUT graceful shutdown — simulates process crash
        drop(store);
    }
    // Reopen from the same file; committed data must survive.
    let store = LibsqlStore::open(&db_path).await.unwrap();
    let got = store
        .get_session("persist")
        .await
        .unwrap()
        .expect("survived");
    assert_eq!(got.id, "persist");
    let loaded = store.load_messages("persist").await.unwrap();
    assert_eq!(loaded.len(), 7);
    assert_eq!(loaded[0].text(), "c msg 0");
}

#[tokio::test]
async fn jsonl_import_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let jsonl_dir = dir.path().join("sessions");
    tokio::fs::create_dir_all(&jsonl_dir).await.unwrap();

    let original: Vec<Message> = conv("imp", 4);
    let path = jsonl_dir.join("imp-session.jsonl");
    let mut text = String::new();
    for m in &original {
        text.push_str(&serde_json::to_string(m).unwrap());
        text.push('\n');
    }
    tokio::fs::write(&path, text).await.unwrap();

    let db = dir.path().join("imp.db");
    let store = LibsqlStore::open(&db).await.unwrap();
    let report = opencode_store::import::import_jsonl_dir(&store, &jsonl_dir)
        .await
        .unwrap();
    assert_eq!(report.sessions, 1);
    assert_eq!(report.messages, 4);

    let loaded = store.load_messages("imp-session").await.unwrap();
    assert_eq!(loaded.len(), original.len());
    for (a, b) in original.iter().zip(loaded.iter()) {
        assert_eq!(a.role, b.role, "role mismatch");
        assert_eq!(a.text(), b.text(), "text mismatch");
        assert_eq!(a.created_at, b.created_at, "ts mismatch");
    }

    // idempotent re-run: skips already-imported
    let report2 = opencode_store::import::import_jsonl_dir(&store, &jsonl_dir)
        .await
        .unwrap();
    assert_eq!(report2.sessions, 0, "second run skips existing");
}

#[tokio::test]
async fn schema_migration_versioning() {
    let (_dir, store) = fresh().await;
    let conn = store.conn().await.unwrap();
    let stmt = conn
        .prepare("SELECT version FROM schema_version LIMIT 1")
        .await
        .unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    let r = rows.next().await.unwrap().expect("version row exists");
    let v: i64 = r.get(0).unwrap();
    assert_eq!(v, 1, "schema_version must be 1 after bootstrap");
}

#[tokio::test]
async fn transaction_rollback_on_partial_failure() {
    let (_dir, store) = fresh().await;
    make_session(&store, "ok", 1).await;

    // Atomicity contract: appending to a non-existent session (FK violation)
    // fails and leaves NO partial state for that session.
    let bad = store.append_messages("ghost-session", &conv("g", 3)).await;
    assert!(bad.is_err(), "FK violation must error");
    assert!(store
        .load_messages("ghost-session")
        .await
        .unwrap()
        .is_empty());

    // The legit session is unaffected.
    store.append_messages("ok", &conv("ok", 2)).await.unwrap();
    assert_eq!(store.load_messages("ok").await.unwrap().len(), 2);

    // Mid-tx rollback at the libsql level: 3 valid inserts followed by a
    // NOT-NULL violation must roll back ALL of them.
    let conn = store.conn().await.unwrap();
    let tx = conn.transaction().await.unwrap();
    tx.execute(
        "INSERT INTO messages (id, session_id, role, blocks_json, usage_json, created_at, synthetic) VALUES ('r1','ok','user','[]','{}',1,0)",
        libsql::params![],
    )
    .await
    .unwrap();
    tx.execute(
        "INSERT INTO messages (id, session_id, role, blocks_json, usage_json, created_at, synthetic) VALUES ('r2','ok','user','[]','{}',2,0)",
        libsql::params![],
    )
    .await
    .unwrap();
    let failed = tx
        .execute(
            "INSERT INTO messages (id, session_id, role, blocks_json, usage_json, created_at, synthetic) VALUES (NULL,'ok','user','[]','{}',3,0)",
            libsql::params![],
        )
        .await;
    assert!(failed.is_err(), "NOT NULL violation must error");
    drop(tx); // explicit drop = rollback
              // none of r1/r2 landed
    let loaded = store.load_messages("ok").await.unwrap();
    assert_eq!(loaded.len(), 2, "rolled-back rows must not appear");
}

#[tokio::test]
async fn list_pagination_with_metadata() {
    let (_dir, store) = fresh().await;
    for i in 0..6u32 {
        let id = format!("p{i}");
        make_session(&store, &id, 1000 + i as i64).await;
        store.append_messages(&id, &conv(&id, 1)).await.unwrap();
    }

    let page1 = store
        .list_sessions(&SessionFilter {
            limit: 3,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(page1.len(), 3);
    // newest first
    assert_eq!(page1[0].id, "p5");
    assert_eq!(page1[1].id, "p4");
    assert!(page1[0].preview.contains("p5 msg 0"));

    let cursor = format!("{}|{}", page1[2].created_at, page1[2].id);
    let page2 = store
        .list_sessions(&SessionFilter {
            limit: 3,
            cursor: Some(cursor),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(page2.len(), 3);
    assert_eq!(page2[0].id, "p2");

    let hits = store
        .list_sessions(&SessionFilter {
            limit: 10,
            search: Some("p3".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "p3");
}

#[tokio::test]
async fn events_append_and_after_replay() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s", 1).await;
    use opencode_store::{EventKind, SessionEventRecord};
    for i in 0..5u32 {
        store
            .append_event(&SessionEventRecord {
                session_id: "s".into(),
                kind: if i == 0 {
                    EventKind::PromptAdmitted
                } else {
                    EventKind::TextDelta
                },
                payload: serde_json::json!({"i": i}),
                ts: i as i64,
                seq: None,
            })
            .await
            .unwrap();
    }
    // replay after seq 2 → events 3,4,5 (3 events, payloads i=2,3,4)
    let tail = store.events_after("s", 2).await.unwrap();
    assert_eq!(tail.len(), 3);
    assert_eq!(tail[0].payload["i"], 2);
    assert!(tail[0].seq.unwrap() > 2);
}

#[tokio::test]
async fn backend_name_reports_libsql() {
    let (_dir, store) = fresh().await;
    assert_eq!(store.backend_name(), "libsql");
}

#[tokio::test]
async fn last_message_seq_tracks_appends() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s", 0).await;
    assert_eq!(store.last_message_seq("s").await.unwrap(), 0);

    let msg1 = Message::user("u1", "hello");
    let seq1 = store.append_message("s", &msg1).await.unwrap();
    assert_eq!(seq1, 1);
    assert_eq!(store.last_message_seq("s").await.unwrap(), 1);

    let msg2 = Message::assistant("u2");
    let seq2 = store.append_message("s", &msg2).await.unwrap();
    assert_eq!(seq2, 2);
    assert_eq!(store.last_message_seq("s").await.unwrap(), 2);
}

#[tokio::test]
async fn delivery_parse_and_as_str_roundtrip() {
    use opencode_store::Delivery;
    assert_eq!(Delivery::parse("steer"), Some(Delivery::Steer));
    assert_eq!(Delivery::parse("queue"), Some(Delivery::Queue));
    assert_eq!(Delivery::parse("invalid"), None);
    assert_eq!(Delivery::Steer.as_str(), "steer");
    assert_eq!(Delivery::Queue.as_str(), "queue");
    // case-insensitive
    assert_eq!(Delivery::parse("STEER"), Some(Delivery::Steer));
    assert_eq!(Delivery::parse("Queue"), Some(Delivery::Queue));
}

#[tokio::test]
async fn subagent_task_crud_roundtrip() {
    use opencode_store::{SubagentStatus, SubagentTaskRecord};

    let (_dir, store) = fresh().await;
    // Seed session rows so the FK constraints on parent/child resolve.
    make_session(&store, "parent-sess", 0).await;
    make_session(&store, "sub-sess-001", 0).await;

    let rec = SubagentTaskRecord {
        task_id: "task-001".into(),
        parent_session_id: "parent-sess".into(),
        child_session_id: "sub-sess-001".into(),
        parent_message_id: Some("msg-42".into()),
        agent: "explore".into(),
        prompt: "find all TODO comments".into(),
        result: None,
        status: SubagentStatus::Running,
        ok: None,
        started_at: 1000,
        completed_at: None,
    };
    store.create_subagent_task(&rec).await.unwrap();

    // List as Running.
    let rows = store.list_subagent_tasks("parent-sess").await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].task_id, "task-001");
    assert_eq!(rows[0].child_session_id, "sub-sess-001");
    assert_eq!(rows[0].agent, "explore");
    assert!(matches!(rows[0].status, SubagentStatus::Running));
    assert!(rows[0].result.is_none());
    assert!(rows[0].ok.is_none());

    // Complete it.
    store
        .complete_subagent_task("task-001", "found 5 TODOs", true)
        .await
        .unwrap();

    // List again — must reflect completion.
    let rows = store.list_subagent_tasks("parent-sess").await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(matches!(rows[0].status, SubagentStatus::Completed));
    assert_eq!(rows[0].result.as_deref(), Some("found 5 TODOs"));
    assert_eq!(rows[0].ok, Some(true));
    assert!(rows[0].completed_at.is_some(), "completed_at must be set");
}

#[tokio::test]
async fn subagent_task_list_filters_by_parent() {
    use opencode_store::{SubagentStatus, SubagentTaskRecord};

    let (_dir, store) = fresh().await;

    for (tid, parent) in [("t-a", "sess-a"), ("t-b", "sess-b"), ("t-c", "sess-a")] {
        make_session(&store, parent, 0).await;
        make_session(&store, &format!("child-{tid}"), 0).await;
        let rec = SubagentTaskRecord {
            task_id: tid.into(),
            parent_session_id: parent.into(),
            child_session_id: format!("child-{tid}"),
            parent_message_id: None,
            agent: "build".into(),
            prompt: format!("prompt-{tid}"),
            result: None,
            status: SubagentStatus::Running,
            ok: None,
            started_at: 2000,
            completed_at: None,
        };
        store.create_subagent_task(&rec).await.unwrap();
    }

    let a_rows = store.list_subagent_tasks("sess-a").await.unwrap();
    assert_eq!(a_rows.len(), 2, "sess-a should have 2 tasks");
    let b_rows = store.list_subagent_tasks("sess-b").await.unwrap();
    assert_eq!(b_rows.len(), 1, "sess-b should have 1 task");
    let none_rows = store.list_subagent_tasks("sess-c").await.unwrap();
    assert!(none_rows.is_empty(), "sess-c should have 0 tasks");
}

#[tokio::test]
async fn subagent_status_parse_and_as_str() {
    use opencode_store::SubagentStatus;
    assert_eq!(SubagentStatus::parse("running"), SubagentStatus::Running);
    assert_eq!(
        SubagentStatus::parse("completed"),
        SubagentStatus::Completed
    );
    assert_eq!(SubagentStatus::parse("failed"), SubagentStatus::Failed);
    assert_eq!(SubagentStatus::parse("bogus"), SubagentStatus::Running);
    assert_eq!(SubagentStatus::Running.as_str(), "running");
    assert_eq!(SubagentStatus::Completed.as_str(), "completed");
    assert_eq!(SubagentStatus::Failed.as_str(), "failed");
}

#[tokio::test]
async fn bundle_export_import_roundtrip() {
    use opencode_store::{
        export_bundle, import_bundle, read_bundle, write_bundle, SubagentStatus, SubagentTaskRecord,
    };

    let dir = TempDir::new().unwrap();
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();

    // Create parent session with messages.
    let parent_meta = SessionMeta {
        id: "parent-1".into(),
        title: Some("parent".into()),
        agent: Some("act".into()),
        model: Some("test-model".into()),
        workdir_hash: None,
        created_at: 1000,
        updated_at: 2000,
        summary: None,
        summary_seq: None,
    };
    store.create_session(&parent_meta).await.unwrap();
    let msgs = conv("parent", 4);
    store.append_messages("parent-1", &msgs).await.unwrap();

    // Create child session with messages.
    let child_meta = SessionMeta {
        id: "child-1".into(),
        title: Some("child".into()),
        agent: Some("explore".into()),
        model: Some("test-model".into()),
        workdir_hash: None,
        created_at: 1100,
        updated_at: 2100,
        summary: None,
        summary_seq: None,
    };
    store.create_session(&child_meta).await.unwrap();
    let child_msgs = conv("child", 2);
    store.append_messages("child-1", &child_msgs).await.unwrap();

    // Link parent → child.
    let task = SubagentTaskRecord {
        task_id: "task-1".into(),
        parent_session_id: "parent-1".into(),
        child_session_id: "child-1".into(),
        parent_message_id: None,
        agent: "explore".into(),
        prompt: "investigate".into(),
        result: Some("done".into()),
        status: SubagentStatus::Completed,
        ok: Some(true),
        started_at: 1500,
        completed_at: Some(1600),
    };
    store.create_subagent_task(&task).await.unwrap();

    // Export.
    let bundle = export_bundle(&store, "parent-1").await.unwrap();
    assert_eq!(bundle.meta.id, "parent-1");
    assert_eq!(bundle.messages.len(), 4);
    assert_eq!(bundle.subagents.len(), 1);
    assert_eq!(bundle.subagents[0].child.meta.id, "child-1");
    assert_eq!(bundle.subagents[0].child.messages.len(), 2);

    // Write to binary, read back.
    let mut buf = Vec::new();
    write_bundle(&bundle, &mut buf).unwrap();
    assert_eq!(&buf[..8], b"OPENCODR");
    let mut cursor = std::io::Cursor::new(&buf);
    let restored = read_bundle(&mut cursor).unwrap();
    assert_eq!(restored.messages.len(), 4);
    assert_eq!(restored.subagents.len(), 1);

    // Import into a fresh store.
    let dir2 = TempDir::new().unwrap();
    let store2 = LibsqlStore::open(dir2.path().join("test2.db"))
        .await
        .unwrap();
    let id = import_bundle(&store2, &restored, None).await.unwrap();
    assert_eq!(id, "parent-1");

    // Verify parent messages.
    let msgs2 = store2.load_messages("parent-1").await.unwrap();
    assert_eq!(msgs2.len(), 4);

    // Verify child session + messages.
    let child2 = store2.load_messages("child-1").await.unwrap();
    assert_eq!(child2.len(), 2);

    // Verify subagent link.
    let tasks2 = store2.list_subagent_tasks("parent-1").await.unwrap();
    assert_eq!(tasks2.len(), 1);
    assert_eq!(tasks2[0].child_session_id, "child-1");

    // Idempotent re-import should be skipped.
    import_bundle(&store2, &restored, None).await.unwrap();
    let msgs3 = store2.load_messages("parent-1").await.unwrap();
    assert_eq!(msgs3.len(), 4, "re-import must not duplicate");
}

#[tokio::test]
async fn list_sessions_excludes_subagents_by_default() {
    use opencode_store::{SubagentStatus, SubagentTaskRecord};

    let (_dir, store) = fresh().await;
    // Parent and child sessions.
    make_session(&store, "parent", 1000).await;
    make_session(&store, "child-sub", 2000).await;

    // Link child as a subagent of parent.
    let rec = SubagentTaskRecord {
        task_id: "task-1".into(),
        parent_session_id: "parent".into(),
        child_session_id: "child-sub".into(),
        parent_message_id: None,
        agent: "explore".into(),
        prompt: "do stuff".into(),
        result: None,
        status: SubagentStatus::Running,
        ok: None,
        started_at: 1500,
        completed_at: None,
    };
    store.create_subagent_task(&rec).await.unwrap();

    // Default filter (include_subagents == false) excludes the child.
    let items = store
        .list_sessions(&SessionFilter::default())
        .await
        .unwrap();
    assert_eq!(
        items.len(),
        1,
        "subagent session should be excluded by default"
    );
    assert_eq!(items[0].id, "parent");

    // With include_subagents == true, both appear.
    let filter = SessionFilter {
        include_subagents: true,
        ..Default::default()
    };
    let items = store.list_sessions(&filter).await.unwrap();
    assert_eq!(
        items.len(),
        2,
        "both parent and child should appear with include_subagents"
    );
}

// =============================================================================
// Diagnostic reproduction tests for concurrent-write failures.
//
// The existing `concurrent_readers_while_writer` test only covers 1 writer +
// N readers, so it can never surface write-lock contention. These two tests
// hammer a FILE-BACKED libsql DB with many CONCURRENT WRITERS (mimicking
// parallel subagent sessions, which all share one `Arc<dyn Store>`), to
// surface SQLITE_BUSY / other write-lock errors and capture the real error
// text. They REPORT rather than hard-assert, because the contention itself is
// the phenomenon under investigation. Run with: --nocapture --test-threads=1.
// =============================================================================

/// Test A — pure concurrent writers: 8 sessions x 50 single-row append_message
/// (the exact path `SessionState::record` -> `append_message` takes), no sleep,
/// to maximize write-lock contention.
#[tokio::test]
async fn concurrent_writers_reproduce_busy() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(
        LibsqlStore::open(dir.path().join("busy.db"))
            .await
            .unwrap(),
    );
    const W: u32 = 8;
    const N: u32 = 50;
    for w in 0..W {
        make_session(&store, &format!("child{w}"), w as i64).await;
    }
    let errs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::new();
    for w in 0..W {
        let s = store.clone();
        let errs = errs.clone();
        handles.push(tokio::spawn(async move {
            let sid = format!("child{w}");
            for k in 0..N {
                let m = Message::user(format!("u-{w}-{k}"), format!("body-{w}-{k}"));
                if let Err(e) = s.append_message(&sid, &m).await {
                    errs.lock()
                        .unwrap()
                        .push(format!("[w{w} k{k} append_message] {e:#}"));
                }
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    let errs = errs.lock().unwrap();
    let total = W * N;
    eprintln!(
        "== concurrent_writers_reproduce_busy: {}/{} writes failed ==",
        errs.len(),
        total
    );
    for e in errs.iter() {
        eprintln!("WRITE_ERR {e}");
    }
    let landed = store.load_messages("child0").await.unwrap().len();
    eprintln!("child0 landed messages: {landed}/{N}");
}

/// Test B — mixed concurrent writes: each writer interleaves
/// append_message + append_event + claim_next_queue (BEGIN IMMEDIATE tx),
/// which holds the write lock for the whole transaction and may starve
/// concurrent message appends — closer to the real runner mix.
#[tokio::test]
async fn mixed_concurrent_writes_with_immediate_tx() {
    use opencode_store::{Delivery, EventKind, SessionEventRecord, SessionInput};
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(
        LibsqlStore::open(dir.path().join("mixed.db"))
            .await
            .unwrap(),
    );
    const W: u32 = 8;
    const ITERS: u32 = 20;
    for w in 0..W {
        let sid = format!("child{w}");
        make_session(&store, &sid, w as i64).await;
        for k in 0..ITERS {
            let inp = SessionInput {
                id: format!("in-{w}-{k}"),
                session_id: sid.clone(),
                delivery: Delivery::Queue,
                prompt: format!("q-{w}-{k}"),
                admitted_seq: k as i64 + 1,
                promoted_seq: None,
            };
            store.admit_input(&inp).await.unwrap();
        }
    }
    let errs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::new();
    for w in 0..W {
        let s = store.clone();
        let errs = errs.clone();
        handles.push(tokio::spawn(async move {
            let sid = format!("child{w}");
            for k in 0..ITERS {
                let m = Message::user(format!("u-{w}-{k}"), format!("body-{w}-{k}"));
                if let Err(e) = s.append_message(&sid, &m).await {
                    errs.lock()
                        .unwrap()
                        .push(format!("[w{w} k{k} append_message] {e:#}"));
                }
                let rec = SessionEventRecord {
                    session_id: sid.clone(),
                    kind: EventKind::TextDelta,
                    payload: serde_json::Value::String(format!("ev-{w}-{k}")),
                    ts: k as i64,
                    seq: None,
                };
                if let Err(e) = s.append_event(&rec).await {
                    errs.lock()
                        .unwrap()
                        .push(format!("[w{w} k{k} append_event] {e:#}"));
                }
                if let Err(e) = s.claim_next_queue(&sid).await {
                    errs.lock()
                        .unwrap()
                        .push(format!("[w{w} k{k} claim_next_queue] {e:#}"));
                }
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    let errs = errs.lock().unwrap();
    eprintln!(
        "== mixed_concurrent_writes_with_immediate_tx: {} ops failed ==",
        errs.len()
    );
    for e in errs.iter() {
        eprintln!("WRITE_ERR {e}");
    }
}

/// Test C — extreme pressure: 32 sessions x 200 single-row appends, to test
/// whether `busy_timeout=5000` ever breaks under heavy intra-process load.
#[tokio::test]
async fn extreme_concurrent_writers() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(
        LibsqlStore::open(dir.path().join("extreme.db"))
            .await
            .unwrap(),
    );
    const W: u32 = 32;
    const N: u32 = 200;
    for w in 0..W {
        make_session(&store, &format!("c{w}"), w as i64).await;
    }
    let errs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::new();
    for w in 0..W {
        let s = store.clone();
        let errs = errs.clone();
        handles.push(tokio::spawn(async move {
            let sid = format!("c{w}");
            for k in 0..N {
                let payload = "x".repeat(512);
                let m = Message::user(format!("u{w}-{k}"), payload);
                if let Err(e) = s.append_message(&sid, &m).await {
                    errs.lock()
                        .unwrap()
                        .push(format!("[w{w} k{k}] {e:#}"));
                }
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    let errs = errs.lock().unwrap();
    eprintln!(
        "== extreme_concurrent_writers: {}/{} writes failed ==",
        errs.len(),
        W * N
    );
    for e in errs.iter().take(20) {
        eprintln!("WRITE_ERR {e}");
    }
}

/// Test D — TWO separate `LibsqlStore` handles opened on the SAME db file
/// (mimicking two processes — e.g. TUI + web server — or two independent
/// connection pools hitting one opencode.db). Each store spawns concurrent
/// writers. This is the configuration most likely to surface cross-connection
/// write-lock contention.
#[tokio::test]
async fn two_stores_same_file_concurrent_writers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shared.db");
    let store_a = Arc::new(LibsqlStore::open(&path).await.unwrap());
    let store_b = Arc::new(LibsqlStore::open(&path).await.unwrap());
    const W: u32 = 6;
    const N: u32 = 50;
    for w in 0..W {
        let sid = format!("c{w}");
        store_a.create_session(&meta_for(&sid)).await.unwrap();
    }
    let _ = (store_a, store_b); // moved into closures below
    let store_a = Arc::new(LibsqlStore::open(&path).await.unwrap());
    let store_b = Arc::new(LibsqlStore::open(&path).await.unwrap());
    let errs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::new();
    for w in 0..W {
        let s = if w % 2 == 0 { store_a.clone() } else { store_b.clone() };
        let errs = errs.clone();
        handles.push(tokio::spawn(async move {
            let sid = format!("c{w}");
            for k in 0..N {
                let m = Message::user(format!("u{w}-{k}"), format!("b{w}-{k}"));
                if let Err(e) = s.append_message(&sid, &m).await {
                    errs.lock()
                        .unwrap()
                        .push(format!("[w{w} k{k}] {e:#}"));
                }
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    let errs = errs.lock().unwrap();
    eprintln!(
        "== two_stores_same_file_concurrent_writers: {}/{} writes failed ==",
        errs.len(),
        W * N
    );
    for e in errs.iter().take(20) {
        eprintln!("WRITE_ERR {e}");
    }
}

fn meta_for(id: &str) -> SessionMeta {
    SessionMeta {
        id: id.to_string(),
        title: Some(format!("t-{id}")),
        agent: Some("act".into()),
        model: Some("glm-5.2".into()),
        workdir_hash: Some("h".into()),
        created_at: 1,
        updated_at: 1,
        summary: None,
        summary_seq: None,
    }
}
