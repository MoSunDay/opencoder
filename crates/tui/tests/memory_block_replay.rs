//! Resume-path contract for local-memory maintenance blocks: the memory run
//! has no child session rows (store-less context copy), so its durable trace
//! is the parent's persisted `subagent_*` frames. `replay_into_chat` must
//! rebuild the foldable block from those frames — including its token spend
//! in the parent `[tok cost]` — matching the live view that folded
//! `SubagentChild(LlmUsage)` in real time.

use std::sync::Arc;

use opencoder_core::Message;
use opencoder_session::SessionEvent;
use opencoder_store::{EventKind, LibsqlStore, SessionEventRecord, SessionMeta, Store};
use opencoder_tui::session_ui::replay_into_chat;
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
        kind: None,
    };
    store.create_session(&meta).await.unwrap();
}

fn assistant_with_usage(id: &str, total_tokens: u64, created_at: i64) -> Message {
    let mut m = Message::assistant(id);
    m.usage.total_tokens = total_tokens;
    m.created_at = created_at;
    m
}

fn user_at(id: &str, text: &str, created_at: i64) -> Message {
    let mut m = Message::user(id, text);
    m.created_at = created_at;
    m
}

/// Persist one parent event in the PRODUCTION sink form (`sse_kind` +
/// `sse_data`) — the same bytes `event_sink`/`persist_event` write.
fn sse_row(session: &str, ev: &SessionEvent, ts: i64) -> SessionEventRecord {
    SessionEventRecord {
        session_id: session.into(),
        kind: EventKind::Step,
        payload: ev.sse_data(),
        ts,
        seq: None,
        sse_kind: Some(ev.sse_kind().to_string()),
    }
}

fn memory_start(id: &str) -> SessionEvent {
    SessionEvent::SubagentStart {
        id: id.into(),
        kind: "memory".into(),
        prompt: "The main task is complete. Update repository local memory.".into(),
        child_session_id: format!("memory-{id}"),
    }
}

fn memory_usage(id: &str, total: u64) -> SessionEvent {
    SessionEvent::SubagentChild {
        id: id.into(),
        ev: Box::new(SessionEvent::LlmUsage {
            total_tokens: total,
            input_tokens: total - 10,
            output_tokens: 10,
        }),
    }
}

fn memory_block(view: &opencoder_tui::chat::ChatView) -> &opencoder_tui::chat::ChatBlock {
    view.blocks
        .iter()
        .find(|b| {
            matches!(b, opencoder_tui::chat::ChatBlock::Subagent { kind, .. } if kind == "memory")
        })
        .expect("a memory block was rebuilt")
}

#[tokio::test]
async fn replay_rebuilds_memory_block_and_folds_usage_into_parent_total() {
    let (dir, store) = fresh().await;
    make_session(&store, "p").await;
    let msgs = vec![
        user_at("u1", "ship it", 100),
        assistant_with_usage("a1", 1_000_000, 200),
    ];
    store.append_messages("p", &msgs).await.unwrap();
    let rows = vec![
        sse_row("p", &memory_start("m1"), 210),
        sse_row(
            "p",
            &SessionEvent::SubagentChild {
                id: "m1".into(),
                ev: Box::new(SessionEvent::TextDelta("polishing notes".into())),
            },
            211,
        ),
        sse_row("p", &memory_usage("m1", 1234), 212),
        sse_row(
            "p",
            &SessionEvent::SubagentEnd {
                id: "m1".into(),
                ok: true,
                cancelled: false,
                summary: "(2s) updated AGENTS.md".into(),
            },
            213,
        ),
    ];
    store.append_events(&rows).await.unwrap();

    let replayed = replay_into_chat("act", &msgs, &store, "p", 0).await;
    // Parent round + maintenance round both land in [tok cost].
    assert_eq!(replayed.tokens_total, 1_001_234);
    match memory_block(&replayed) {
        opencoder_tui::chat::ChatBlock::Subagent {
            done,
            ok,
            summary,
            view,
            ..
        } => {
            assert!(*done && *ok, "the persisted run closed ok");
            assert!(summary.starts_with("(2s)"), "summary={summary}");
            assert_eq!(view.tokens_total, 1234, "child view keeps its own spend");
        }
        other => panic!("unexpected block: {other:?}"),
    }
    let _ = dir;
}

#[tokio::test]
async fn memory_blocks_interleave_between_tasks() {
    let (dir, store) = fresh().await;
    make_session(&store, "p").await;
    let msgs = vec![
        user_at("u1", "task one", 100),
        assistant_with_usage("a1", 100_000, 200),
        user_at("u2", "task two", 1_000),
        assistant_with_usage("a2", 200_000, 1_100),
    ];
    store.append_messages("p", &msgs).await.unwrap();
    let rows = vec![
        sse_row("p", &memory_start("m1"), 250),
        sse_row("p", &memory_usage("m1", 1_000), 251),
        sse_row(
            "p",
            &SessionEvent::SubagentEnd {
                id: "m1".into(),
                ok: true,
                cancelled: false,
                summary: "(1s) done".into(),
            },
            252,
        ),
        sse_row("p", &memory_start("m2"), 1_150),
        sse_row("p", &memory_usage("m2", 2_000), 1_151),
        sse_row(
            "p",
            &SessionEvent::SubagentEnd {
                id: "m2".into(),
                ok: true,
                cancelled: false,
                summary: "(1s) done".into(),
            },
            1_152,
        ),
    ];
    store.append_events(&rows).await.unwrap();

    let replayed = replay_into_chat("act", &msgs, &store, "p", 0).await;
    // 100k + 200k parent rounds + 1k + 2k maintenance rounds.
    assert_eq!(replayed.tokens_total, 303_000);
    let positions: Vec<usize> = replayed
        .blocks
        .iter()
        .enumerate()
        .filter_map(|(i, b)| {
            matches!(b, opencoder_tui::chat::ChatBlock::Subagent { id, .. } if id == "m1" || id == "m2")
                .then_some(i)
        })
        .collect();
    assert_eq!(positions.len(), 2, "both memory blocks rebuilt");
    let (m1, m2) = (positions[0], positions[1]);
    // The second task's user boundary must sit strictly between the two
    // memory blocks: m1 after task one, m2 after task two.
    let u2 = replayed
        .blocks
        .iter()
        .position(|b| {
            matches!(b, opencoder_tui::chat::ChatBlock::User { rendered } if
                rendered.iter().any(|l| l.to_string().contains("task two")))
        })
        .expect("second task's user boundary");
    assert!(
        m1 < u2 && u2 < m2,
        "m1={m1} u2={u2} m2={m2}: blocks interleaved"
    );
    let _ = dir;
}

#[tokio::test]
async fn truncated_memory_run_shows_interrupted_and_keeps_usage() {
    let (dir, store) = fresh().await;
    make_session(&store, "p").await;
    let msgs = vec![
        user_at("u1", "go", 100),
        assistant_with_usage("a1", 500_000, 200),
    ];
    store.append_messages("p", &msgs).await.unwrap();
    // Enum-form payload (the `resume.rs` writer shape) pins the fallback
    // parse path; no `SubagentEnd` — the log ends mid-run.
    let start = memory_start("m9");
    let rows = vec![
        SessionEventRecord {
            session_id: "p".into(),
            kind: EventKind::Step,
            payload: serde_json::to_value(&start).unwrap(),
            ts: 210,
            seq: None,
            sse_kind: Some(start.sse_kind().to_string()),
        },
        SessionEventRecord {
            session_id: "p".into(),
            kind: EventKind::Step,
            payload: serde_json::to_value(memory_usage("m9", 777)).unwrap(),
            ts: 211,
            seq: None,
            sse_kind: Some(memory_usage("m9", 777).sse_kind().to_string()),
        },
    ];
    store.append_events(&rows).await.unwrap();

    let replayed = replay_into_chat("act", &msgs, &store, "p", 0).await;
    assert_eq!(replayed.tokens_total, 500_777, "truncated run still pays");
    match memory_block(&replayed) {
        opencoder_tui::chat::ChatBlock::Subagent {
            done, ok, summary, ..
        } => {
            assert!(*done && !*ok && *summary == "(interrupted)");
        }
        other => panic!("unexpected block: {other:?}"),
    }
    let _ = dir;
}

#[tokio::test]
async fn replayed_memory_cost_matches_the_live_view() {
    let (dir, store) = fresh().await;
    make_session(&store, "p").await;
    let msgs = vec![
        user_at("u1", "parity", 100),
        assistant_with_usage("a1", 800_000, 200),
    ];
    store.append_messages("p", &msgs).await.unwrap();
    let rows = vec![
        sse_row("p", &memory_start("m1"), 210),
        sse_row("p", &memory_usage("m1", 1_500), 211),
        sse_row(
            "p",
            &SessionEvent::SubagentEnd {
                id: "m1".into(),
                ok: true,
                cancelled: false,
                summary: "(1s) done".into(),
            },
            212,
        ),
    ];
    store.append_events(&rows).await.unwrap();

    // Live: the same frames a running session broadcasts.
    let mut live = opencoder_tui::chat::ChatView::default();
    live.apply(&SessionEvent::LlmUsage {
        total_tokens: 800_000,
        input_tokens: 700_000,
        output_tokens: 100_000,
    });
    live.apply(&memory_start("m1"));
    live.apply(&memory_usage("m1", 1_500));
    live.apply(&SessionEvent::SubagentEnd {
        id: "m1".into(),
        ok: true,
        cancelled: false,
        summary: "(1s) done".into(),
    });

    let replayed = replay_into_chat("act", &msgs, &store, "p", 0).await;
    assert_eq!(replayed.tokens_total, live.tokens_total);
    let _ = dir;
}
