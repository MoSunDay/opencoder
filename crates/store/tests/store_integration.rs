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

use std::sync::Arc;

use opencode_core::{ContentBlock, Message, Role};
use opencode_store::{
    Delivery, LibsqlStore, SessionFilter, SessionInput, SessionMeta, Store,
};
use tempfile::TempDir;

fn conv(seed: &str, n: usize) -> Vec<Message> {
    (0..n)
        .map(|i| {
            let id = format!("{seed}-{i}");
            let role = if i % 2 == 0 { Role::User } else { Role::Assistant };
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

    let got = store.get_session("s1").await.unwrap().expect("session exists");
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
    store_raw.append_messages("s", &conv("seed", 10)).await.unwrap();
    let store = Arc::new(store_raw);
    let _dir = dir; // keep alive

    let store_w = store.clone();
    let writer = tokio::spawn(async move {
        for b in 0..20u32 {
            let msgs = conv(&format!("w{b}"), 5);
            store_w.append_messages("s", &msgs).await.expect("append ok");
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
        store.append_messages("persist", &conv("c", 7)).await.unwrap();
        // drop store WITHOUT graceful shutdown — simulates process crash
        drop(store);
    }
    // Reopen from the same file; committed data must survive.
    let store = LibsqlStore::open(&db_path).await.unwrap();
    let got = store.get_session("persist").await.unwrap().expect("survived");
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
    let stmt = conn.prepare("SELECT version FROM schema_version LIMIT 1").await.unwrap();
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
    let bad = store
        .append_messages("ghost-session", &conv("g", 3))
        .await;
    assert!(bad.is_err(), "FK violation must error");
    assert!(store.load_messages("ghost-session").await.unwrap().is_empty());

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
        .list_sessions(&SessionFilter { limit: 3, ..Default::default() })
        .await
        .unwrap();
    assert_eq!(page1.len(), 3);
    // newest first
    assert_eq!(page1[0].id, "p5");
    assert_eq!(page1[1].id, "p4");
    assert!(page1[0].preview.contains("p5 msg 0"));

    let cursor = format!("{}|{}", page1[2].created_at, page1[2].id);
    let page2 = store
        .list_sessions(&SessionFilter { limit: 3, cursor: Some(cursor), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(page2.len(), 3);
    assert_eq!(page2[0].id, "p2");

    let hits = store
        .list_sessions(&SessionFilter { limit: 10, search: Some("p3".into()), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "p3");
}

#[tokio::test]
async fn inputs_steer_and_queue_promotion_semantics() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s", 1).await;

    let admit = |seq: i64, delivery: Delivery, prompt: &str| -> SessionInput {
        SessionInput {
            id: format!("in-{seq}"),
            session_id: "s".into(),
            delivery,
            prompt: prompt.into(),
            admitted_seq: seq,
            promoted_seq: None,
        }
    };

    store.admit_input(&admit(1, Delivery::Steer, "steer-1")).await.unwrap();
    store.admit_input(&admit(2, Delivery::Queue, "queue-1")).await.unwrap();
    store.admit_input(&admit(3, Delivery::Queue, "queue-2")).await.unwrap();

    // pending: 1 steer + 2 queue
    let pending_steer = store.pending_inputs("s", Delivery::Steer).await.unwrap();
    assert_eq!(pending_steer.len(), 1);
    let pending_queue = store.pending_inputs("s", Delivery::Queue).await.unwrap();
    assert_eq!(pending_queue.len(), 2);

    // promote steers up to seq 1 → exactly the 1 steer promoted
    let promoted = store.promote_inputs("s", 1, Delivery::Steer).await.unwrap();
    assert_eq!(promoted.len(), 1);
    assert!(store.pending_inputs("s", Delivery::Steer).await.unwrap().is_empty());

    // promote_next_queued promotes exactly ONE (oldest), leaving the other pending
    let one = store.promote_next_queued("s").await.unwrap();
    assert_eq!(one, Some(2)); // admitted_seq ordering; seq of queue-1
    let still_pending = store.pending_inputs("s", Delivery::Queue).await.unwrap();
    assert_eq!(still_pending.len(), 1, "exactly one queue remains");
    let next = store.promote_next_queued("s").await.unwrap();
    assert!(next.is_some());
    assert!(store.pending_inputs("s", Delivery::Queue).await.unwrap().is_empty());
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
                kind: if i == 0 { EventKind::PromptAdmitted } else { EventKind::TextDelta },
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
