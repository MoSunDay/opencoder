//! Integration test for the `[tok cost]` data path across the store:
//! `replay_into_chat` sums persisted assistant usage, and the
//! `preserve_tokens_total` floor keeps the lifetime accumulation from
//! regressing when a `TranscriptReset` (compaction) rebuild drops the
//! pre-compaction messages.

use std::sync::Arc;

use opencoder_core::Message;
use opencoder_session::SessionEvent;
use opencoder_store::{EventKind, LibsqlStore, SessionEventRecord, SessionMeta, Store};
use opencoder_tui::session_ui::{rebuild_after_reset, replay_into_chat};
use tempfile::TempDir;

async fn fresh() -> (TempDir, Arc<dyn Store>) {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();
    (dir, Arc::new(store) as Arc<dyn Store>)
}

async fn make_session(store: &Arc<dyn Store>, id: &str) {
    let meta = SessionMeta {
        id: id.to_string(),
        title: Some(format!("title-{id}")),
        agent: Some("act".into()),
        model: Some("m".into()),
        autopilot_mode: None,
        workdir_hash: None,
        created_at: 1000,
        updated_at: 1000,
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

fn assistant_with_usage(id: &str, total_tokens: u64) -> Message {
    let mut m = Message::assistant(id);
    m.usage.total_tokens = total_tokens;
    m
}

#[tokio::test]
async fn replay_sums_real_usage_from_persisted_messages() {
    let (dir, store) = fresh().await;
    make_session(&store, "s1").await;
    let msgs = vec![
        Message::user("u1", "hi"),
        assistant_with_usage("a1", 1_000_000),
        assistant_with_usage("a2", 250_000),
    ];
    store.append_messages("s1", &msgs).await.unwrap();
    let reloaded = store.load_messages("s1").await.unwrap();

    // Mirror the real resume path: replay the store-loaded message list.
    let chat = replay_into_chat("act", &reloaded, &store, "s1", 0).await;
    let _ = dir;
    assert_eq!(chat.tokens_total, 1_250_000);
}

#[tokio::test]
async fn preserve_floor_keeps_lifetime_total_across_transcript_reset() {
    let (dir, store) = fresh().await;
    make_session(&store, "s2").await;
    // Post-compaction message list: only the synthetic summary remains, whose
    // usage is zero — the raw sum alone would regress the displayed cost.
    let post_compaction = vec![Message::user("u1", "<summary>")];
    store.append_messages("s2", &post_compaction).await.unwrap();

    // Simulate the live view that had accumulated 2.4m tokens before the
    // reset, then rebuild exactly as the app loop does.
    let mut live = opencoder_tui::chat::ChatView::default();
    live.apply(&SessionEvent::LlmUsage {
        total_tokens: 2_400_000,
        input_tokens: 2_000_000,
        output_tokens: 400_000,
    });
    rebuild_after_reset(&mut live, &post_compaction, &store, "s2").await;

    assert_eq!(
        live.tokens_total, 2_400_000,
        "rebuild must floor the shrunken replay sum with the live accumulation"
    );
    let _ = dir;
}

#[tokio::test]
async fn child_view_replay_accumulates_llm_usage_events() {
    let (dir, store) = fresh().await;
    make_session(&store, "child").await;
    let mk = |total: u64| {
        let ev = SessionEvent::LlmUsage {
            total_tokens: total,
            input_tokens: total / 2,
            output_tokens: total / 2,
        };
        SessionEventRecord {
            session_id: "child".into(),
            kind: EventKind::Step,
            payload: serde_json::to_value(&ev).unwrap(),
            ts: 1,
            seq: None,
            sse_kind: None,
        }
    };
    store
        .append_events(&[mk(400_000), mk(150_000)])
        .await
        .unwrap();

    // `replay_into_chat` replays messages, but the events path
    // (`reconstruct_child_view`) must see the same accumulation: verify via
    // the public ChatView apply used by that path.
    let mut view = opencoder_tui::chat::ChatView::default();
    for rec in store.events_after("child", 0).await.unwrap() {
        let ev: SessionEvent = serde_json::from_value(rec.payload).unwrap();
        view.apply(&ev);
    }
    assert_eq!(view.tokens_total, 550_000);
    let _ = dir;
}

/// Subagent spend must land in the parent's `[tok cost]` on resume exactly as
/// it does live: parent message usage Σ + each child's own total (events
/// path AND messages fallback path), with the live `preserve` floor never
/// double-counting (`max` of two equal values).
#[tokio::test]
async fn replay_folds_subagent_usage_into_parent_total_matching_live() {
    use opencoder_store::{SubagentStatus, SubagentTaskRecord};

    let (dir, store) = fresh().await;
    make_session(&store, "p").await;
    make_session(&store, "c1").await; // events path (primary)
    make_session(&store, "c2").await; // messages path (fallback)

    // Parent transcript: two usage-carrying rounds (Σ = 1_250_000).
    let msgs = vec![
        Message::user("u1", "delegate this"),
        assistant_with_usage("a1", 1_000_000),
        assistant_with_usage("a2", 250_000),
    ];
    store.append_messages("p", &msgs).await.unwrap();

    // Child 1: persisted LlmUsage events (primary reconstruction path).
    let ev_rec = |total: u64| SessionEventRecord {
        session_id: "c1".into(),
        kind: EventKind::Step,
        payload: serde_json::to_value(&SessionEvent::LlmUsage {
            total_tokens: total,
            input_tokens: total - 10,
            output_tokens: 10,
        })
        .unwrap(),
        ts: 1,
        seq: None,
        sse_kind: None,
    };
    store
        .append_events(&[ev_rec(100_000), ev_rec(50_000)])
        .await
        .unwrap();

    // Child 2: usage only on persisted messages (fallback path).
    store
        .append_messages("c2", &[assistant_with_usage("ca1", 50_000)])
        .await
        .unwrap();

    let task = |task_id: &str, child: &str, parent_msg: &str| SubagentTaskRecord {
        task_id: task_id.into(),
        parent_session_id: "p".into(),
        child_session_id: child.into(),
        parent_message_id: Some(parent_msg.into()),
        agent: "explore".into(),
        prompt: "find things".into(),
        result: None,
        status: SubagentStatus::Running,
        ok: None,
        started_at: 1100,
        completed_at: None,
    };
    store
        .create_subagent_task(&task("t1", "c1", "a2"))
        .await
        .unwrap();
    store
        .create_subagent_task(&task("t2", "c2", "a2"))
        .await
        .unwrap();

    let reloaded = store.load_messages("p").await.unwrap();
    let replayed = replay_into_chat("act", &reloaded, &store, "p", 0).await;
    assert_eq!(
        replayed.tokens_total,
        1_250_000 + 150_000 + 50_000,
        "parent total = parent Σ + child Σ (events path + fallback path)"
    );

    // Live-path parity: the runner forwards child rounds as
    // SubagentChild(LlmUsage), which the view folds in the same way.
    let mut live = opencoder_tui::chat::ChatView::default();
    live.apply(&SessionEvent::LlmUsage {
        total_tokens: 1_000_000,
        input_tokens: 900_000,
        output_tokens: 100_000,
    });
    live.apply(&SessionEvent::LlmUsage {
        total_tokens: 250_000,
        input_tokens: 200_000,
        output_tokens: 50_000,
    });
    for (id, total) in [("t1", 150_000u64), ("t2", 50_000u64)] {
        live.apply(&SessionEvent::SubagentChild {
            id: id.into(),
            ev: Box::new(SessionEvent::LlmUsage {
                total_tokens: total,
                input_tokens: total - 10,
                output_tokens: 10,
            }),
        });
    }
    assert_eq!(live.tokens_total, 1_450_000);

    // Resume with the live accumulation as the preserve floor: `max` of two
    // equal values — the totals match and nothing is counted twice.
    let resumed = replay_into_chat("act", &reloaded, &store, "p", live.tokens_total).await;
    assert_eq!(
        resumed.tokens_total, 1_450_000,
        "replay and live must agree; the floor must not double-count"
    );
    let _ = dir;
}

/// End-to-end through the REAL runner: mock model + `task` tool round whose
/// child round carries usage. Applying the emitted event stream to a ChatView
/// must show the parent `[tok cost]` jumping by the child round, while the
/// parent's real ctx tracks only its own last round.
#[tokio::test]
async fn e2e_mock_task_round_folds_child_usage_into_parent_view() {
    use opencoder_core::{resolve_agent, Config};
    use opencoder_llm::{CompletedToolCall, LlmEvent, MockChatClient, Usage};
    use opencoder_session::{run, SessionState};

    let done = |text: &str, total: u64, inp: u64, out: u64| LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: Some(Usage {
            input_tokens: inp,
            output_tokens: out,
            total_tokens: total,
            ..Default::default()
        }),
    };
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![LlmEvent::Completed {
                text: "delegating".into(),
                tool_calls: vec![CompletedToolCall {
                    id: "task-1".into(),
                    name: "task".into(),
                    input: serde_json::json!({
                        "prompt": "research",
                        "subagent_type": "explore"
                    }),
                }],
                usage: Some(Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    total_tokens: 15,
                    ..Default::default()
                }),
            }])
            // Child round: 300 tokens (250 in / 50 out).
            .push_script(vec![done("found it", 300, 250, 50)])
            // Parent closing round: 100 tokens (80 in / 20 out).
            .push_script(vec![done("all done", 100, 80, 20)]),
    );

    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let cfg = Config {
        model: "m/g".into(),
        ..Config::default()
    };
    let mut session = SessionState::new("e2e-tok-cost", agent, cfg, mock, dir.path().to_path_buf());
    let mut events = Vec::new();
    run(&mut session, "delegate".into(), |ev| events.push(ev))
        .await
        .unwrap();

    let mut view = opencoder_tui::chat::ChatView::default();
    for ev in &events {
        view.apply(ev);
    }
    assert_eq!(
        view.tokens_total,
        15 + 300 + 100,
        "parent cost = parent rounds + child round"
    );
    assert_eq!(
        view.real_context_tokens,
        Some(100),
        "parent real ctx = its own last round, never the child's"
    );
    match view
        .blocks
        .iter()
        .find(|b| matches!(b, opencoder_tui::chat::ChatBlock::Subagent { .. }))
    {
        Some(opencoder_tui::chat::ChatBlock::Subagent { view: child, .. }) => {
            assert_eq!(child.tokens_total, 300);
            assert_eq!(child.real_context_tokens, Some(300));
        }
        other => panic!("expected subagent block, got {other:?}"),
    }
}

/// ctx (used/limit) follows the LLM-returned `total_tokens` verbatim, even
/// when it differs from `input_tokens + output_tokens` (e.g. cached or
/// reasoning tokens included in total).
#[tokio::test]
async fn replay_real_context_uses_total_tokens_not_input_plus_output() {
    let (dir, store) = fresh().await;
    make_session(&store, "s-total").await;

    let mut a = Message::assistant("a1");
    a.usage.input_tokens = 10;
    a.usage.output_tokens = 5;
    a.usage.total_tokens = 42;
    let msgs = vec![Message::user("u1", "hi"), a];

    let chat = replay_into_chat("act", &msgs, &store, "s-total", 0).await;

    assert_eq!(chat.real_context_tokens, Some(42));
    let _ = dir;
}

/// Old persisted `LlmUsage` event payloads predate the split fields and carry
/// only `total_tokens` (input/output deserialize to 0 via `#[serde(default)]`).
/// Applying such an event must still set provider truth from the total.
#[tokio::test]
async fn old_usage_event_payload_with_only_total_rebuilds_real_context() {
    let (dir, store) = fresh().await;
    make_session(&store, "s-old-event").await;

    let rec = SessionEventRecord {
        session_id: "s-old-event".into(),
        kind: EventKind::Step,
        payload: serde_json::json!({"LlmUsage": {"total_tokens": 42}}),
        ts: 1,
        seq: None,
        sse_kind: None,
    };
    store.append_events(&[rec]).await.unwrap();

    let mut view = opencoder_tui::chat::ChatView::default();
    for rec in store.events_after("s-old-event", 0).await.unwrap() {
        let ev: SessionEvent = serde_json::from_value(rec.payload).unwrap();
        view.apply(&ev);
    }
    assert_eq!(view.real_context_tokens, Some(42));
    assert_eq!(view.tokens_total, 42);
    let _ = dir;
}
