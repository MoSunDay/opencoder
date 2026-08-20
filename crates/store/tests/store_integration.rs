//! P0 functional tests for the libsql-backed Store.
//!
//! Each test asserts a *behavior contract*, not "the function runs":
//! - create_get_update_delete_session_contract: full CRUD lifecycle
//! - clear_other_sessions_keeps_current_and_cascades: keep-one cleanup + FK cascade
//! - append_and_load_preserves_all_roles_and_blocks: roles/blocks/usage round-trip
//! - jsonl_import_roundtrip: import preserves message history + idempotent re-run
//! - transaction_rollback_on_partial_failure: failed batch leaves no partial rows
//! - list_pagination_with_metadata: cursor pagination + search filter
//! - bundle_export_import_roundtrip: binary bundle export/import incl. subagents
//! - session_handoff_and_skill_fields_round_trip: v3 session fields via patch
//! - cancelled_transaction_*: future-cancellation must not panic and the store
//!   stays usable/consistent afterwards
//!
//! These run against a real on-disk libsql file (tempdir) so WAL behaviour is
//! exercised truthfully, not mocked. Concurrent-writer stress tests live in
//! `store_concurrency.rs`, schema-migration tests in `store_migrations.rs`, and
//! subagent-task tests in `subagent_status_counts.rs`.

use std::sync::Arc;

use opencoder_core::{ContentBlock, Message, Role};
use opencoder_store::{LibsqlStore, SessionFilter, SessionMeta, SessionPatch, Store};
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
        autopilot_mode: None,
        workdir_hash: Some("h".into()),
        created_at: now,
        updated_at: now,
        summary: None,
        summary_seq: None,
        summary_images: vec![],
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
        requirement: None,
        plan_snapshot: None,
        plan_input_count: 0,
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

    let patch = opencoder_store::SessionPatch {
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

/// v11: the session-scoped `autopilot_mode` column must survive the full
/// create -> patch -> clear lifecycle, mirroring how `model` is treated.
#[tokio::test]
async fn autopilot_mode_column_round_trips() {
    let (_dir, store) = fresh().await;
    let meta = SessionMeta {
        id: "s-ap".into(),
        autopilot_mode: Some("ap".into()),
        ..Default::default()
    };
    store.create_session(&meta).await.unwrap();
    assert_eq!(
        store
            .get_session("s-ap")
            .await
            .unwrap()
            .unwrap()
            .autopilot_mode
            .as_deref(),
        Some("ap"),
        "created autopilot_mode must round-trip"
    );

    store
        .update_session(
            "s-ap",
            &opencoder_store::SessionPatch {
                autopilot_mode: Some("review".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .get_session("s-ap")
            .await
            .unwrap()
            .unwrap()
            .autopilot_mode
            .as_deref(),
        Some("review"),
        "patched autopilot_mode must round-trip"
    );

    store
        .update_session(
            "s-ap",
            &opencoder_store::SessionPatch {
                clear_autopilot_mode: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .get_session("s-ap")
            .await
            .unwrap()
            .unwrap()
            .autopilot_mode,
        None,
        "clear_autopilot_mode must NULL the column"
    );
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
            m.usage = opencoder_core::MessageUsage {
                input_tokens: 10,
                output_tokens: 5,
                total_tokens: 15,
                cache_read_tokens: 104_857_600,
                cache_creation_tokens: 1_500,
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
                    images: Vec::new(),
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
    // Cache tokens must survive the SQLite JSON round-trip (usage_json column).
    assert_eq!(loaded[1].usage.cache_read_tokens, 104_857_600);
    assert_eq!(loaded[1].usage.cache_creation_tokens, 1_500);
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
    let report = opencoder_store::import::import_jsonl_dir(&store, &jsonl_dir)
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
    let report2 = opencoder_store::import::import_jsonl_dir(&store, &jsonl_dir)
        .await
        .unwrap();
    assert_eq!(report2.sessions, 0, "second run skips existing");
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
    use opencoder_store::{EventKind, SessionEventRecord};
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
                sse_kind: None,
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
    use opencoder_store::Delivery;
    assert_eq!(Delivery::parse("steer"), Some(Delivery::Steer));
    assert_eq!(Delivery::parse("queue"), Some(Delivery::Queue));
    assert_eq!(Delivery::parse("invalid"), None);
    assert_eq!(Delivery::Steer.as_str(), "steer");
    assert_eq!(Delivery::Queue.as_str(), "queue");
    // case-insensitive
    assert_eq!(Delivery::parse("STEER"), Some(Delivery::Steer));
    assert_eq!(Delivery::parse("Queue"), Some(Delivery::Queue));
    // whitespace-tolerant (a padded " queue " must not degrade to Steer)
    assert_eq!(Delivery::parse("  queue  "), Some(Delivery::Queue));
    assert_eq!(Delivery::parse("\tSTEER\n"), Some(Delivery::Steer));
    assert_eq!(Delivery::parse("   "), None);
    assert_eq!(Delivery::parse(" stear "), None, "a typo must stay invalid");
}

#[tokio::test]
async fn bundle_export_import_roundtrip() {
    use opencoder_store::{
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
        autopilot_mode: None,
        workdir_hash: None,
        created_at: 1000,
        updated_at: 2000,
        summary: None,
        summary_seq: None,
        summary_images: vec![],
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
        requirement: None,
        plan_snapshot: None,
        plan_input_count: 0,
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
        autopilot_mode: None,
        workdir_hash: None,
        created_at: 1100,
        updated_at: 2100,
        summary: None,
        summary_seq: None,
        summary_images: vec![],
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
        requirement: None,
        plan_snapshot: None,
        plan_input_count: 0,
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
    use opencoder_store::{SubagentStatus, SubagentTaskRecord};

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

#[tokio::test]
async fn list_sessions_carries_skill_body_for_picker_tag() {
    let (_dir, store) = fresh().await;
    // Session with an active skill: the store persists the body, not the name.
    store
        .create_session(&SessionMeta {
            id: "skilled".into(),
            agent: Some("plan".into()),

            autopilot_mode: None,
            skill: Some("## do-and-done\nfull body".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    store
        .create_session(&SessionMeta {
            id: "plain".into(),
            agent: Some("act".into()),

            autopilot_mode: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let items = store
        .list_sessions(&SessionFilter::default())
        .await
        .unwrap();
    let skilled = items
        .iter()
        .find(|s| s.id == "skilled")
        .expect("skilled row");
    assert_eq!(
        skilled.skill.as_deref(),
        Some("## do-and-done\nfull body"),
        "list() must surface the stored skill body so the /task picker can tag it"
    );
    let plain = items.iter().find(|s| s.id == "plain").expect("plain row");
    assert_eq!(plain.skill, None, "sessions without a skill must list None");
}

#[tokio::test]
async fn session_handoff_and_skill_fields_round_trip() {
    let store = LibsqlStore::open_memory().await.unwrap();
    let id = "rt-session";
    store
        .create_session(&SessionMeta {
            id: id.into(),
            title: None,
            agent: Some("act".into()),
            model: Some("m".into()),
            autopilot_mode: None,
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
            plan_snapshot: None,
            plan_input_count: 0,
        })
        .await
        .unwrap();

    // Initially null.
    let m0 = store.get_session(id).await.unwrap().unwrap();
    assert!(m0.handoff_seq.is_none());
    assert!(m0.handoff_plan.is_none());
    assert!(m0.skill.is_none());

    // Persist via SessionPatch.
    store
        .update_session(
            id,
            &SessionPatch {
                handoff_seq: Some(7),
                handoff_plan: Some("## Plan\n1. x".into()),
                skill: Some("be terse".into()),
                updated_at: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let m1 = store.get_session(id).await.unwrap().unwrap();
    assert_eq!(m1.handoff_seq, Some(7));
    assert_eq!(m1.handoff_plan.as_deref(), Some("## Plan\n1. x"));
    assert_eq!(m1.skill.as_deref(), Some("be terse"));
    // Untouched fields preserved.
    assert_eq!(m1.agent.as_deref(), Some("act"));
    assert!(m1.summary_seq.is_none());
}

// ---------------------------------------------------------------------------
// Regression: future cancellation must not panic (no libsql::Transaction::Drop)
// ---------------------------------------------------------------------------
//
// Before the fix, every transaction used `libsql::Transaction` whose `Drop`
// calls `do_rollback().unwrap()`. When a future was cancelled mid-transaction
// (e.g. via `tokio::select!`), the `db_lock` guard could be released before
// the `Transaction` was dropped, allowing another task to mutate the shared
// connection and invalidate the transaction state — causing the Drop's
// `unwrap()` to panic the entire process.
//
// With manual BEGIN/COMMIT/ROLLBACK (run_tx), cancellation leaves at worst a
// dangling transaction that the next run_tx recovers from via a pre-BEGIN
// ROLLBACK. No panic, no crash, no data corruption.

#[tokio::test]
async fn cancelled_transaction_does_not_panic() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s1", 1).await;

    // Start a multi-message append (opens a transaction) and cancel it after
    // a tiny delay — simulating tokio::select! interrupting a drain step.
    let big_batch = conv("cancel", 50);
    let store = Arc::new(store);

    let cancelled = {
        let s = store.clone();
        tokio::select! {
            // Bias toward the timeout so the append future starts but gets
            // dropped before (or shortly after) it can commit.
            _ = tokio::time::sleep(std::time::Duration::from_millis(1)) => true,
            res = s.append_messages("s1", &big_batch) => {
                // If it managed to commit, that's fine too — the point is no
                // panic.
                let _ = res;
                false
            }
        }
    };

    // Regardless of whether the cancelled batch committed, the store MUST be
    // usable afterwards without panicking or erroring.
    let follow_up = conv("after", 3);
    store.append_messages("s1", &follow_up).await.unwrap();

    // The follow-up messages must be present and correct.
    let loaded = store.load_messages("s1").await.unwrap();
    let _ = cancelled; // unused in assertions — we just needed the drop to happen
    assert!(
        loaded.iter().any(|m| m.id == "after-0"),
        "post-cancellation append must be persisted"
    );
    // If the cancelled batch committed, there may be up to 50 + 3 = 53 rows.
    // If it was dropped mid-transaction, the dangling tx is rolled back by
    // the next run_tx, so only the 3 follow-up rows exist. Either way, the
    // 3 follow-up messages must all be present.
    for i in 0..3 {
        let id = format!("after-{i}");
        assert!(
            loaded.iter().any(|m| m.id == id),
            "follow-up message {id} must be present"
        );
    }
}

#[tokio::test]
async fn cancelled_then_concurrent_ops_stay_consistent() {
    // Stress variant: cancel several transaction futures interleaved with
    // successful operations, then verify final state is consistent.
    let (_dir, store) = fresh().await;
    make_session(&store, "sx", 1).await;
    let store = Arc::new(store);

    const ROUNDS: usize = 10;

    for round in 0..ROUNDS {
        // Cancel a batch.
        let batch = conv(&format!("c{round}"), 5);
        let s = store.clone();
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_micros(100)) => {}
            _ = s.append_messages("sx", &batch) => {}
        }

        // Immediately do a successful append — must not panic.
        let ok = conv(&format!("ok{round}"), 1);
        store.append_messages("sx", &ok).await.unwrap();
    }

    // All 10 "ok" messages must be present.
    let loaded = store.load_messages("sx").await.unwrap();
    for round in 0..ROUNDS {
        let id = format!("ok{round}-0");
        assert!(
            loaded.iter().any(|m| m.id == id),
            "ok message {id} from round {round} must survive"
        );
    }
}
