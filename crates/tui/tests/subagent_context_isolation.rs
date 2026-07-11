//! End-to-end isolation test for the parent-ctx%-excludes-subagent fix.
//!
//! A real `session::run` dispatches a subagent (task tool). The resulting
//! `SessionEvent` stream — which includes `SubagentChild` wrappers carrying the
//! child's TextDelta/Tool events — is replayed into a `ChatView` exactly as the
//! TUI worker does. The parent view's `context_used` must NOT grow while child
//! events stream in; only the child ChatView (nested in the Subagent block)
//! accumulates its own tokens.
//!
//! This automates what a manual TUI session would show: the parent window's
//! ctx% stays flat while a subagent runs, then the child view carries its own
//! count (visible via ctx-switch).

use std::sync::Arc;

use opencode_core::{resolve_agent, Config};
use opencode_llm::{CompletedToolCall, LlmEvent, MockChatClient};
use opencode_session::{run, SessionEvent, SessionState};
use opencode_tui::chat::{ChatBlock, ChatView};

fn task_turn(prompt: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: "delegating".into(),
        tool_calls: vec![CompletedToolCall {
            id: "task-1".into(),
            name: "task".into(),
            input: serde_json::json!({"prompt": prompt, "subagent_type": "explore"}),
        }],
        usage: None,
    }
}

fn text_done(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
    }
}

#[tokio::test]
async fn real_subagent_stream_does_not_inflate_parent_context() {
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![task_turn("research the codebase")])
            .push_script(vec![
                LlmEvent::TextDelta("child secret conclusion alpha beta gamma".into()),
                text_done(""),
            ])
            .push_script(vec![text_done("parent final answer")]),
    );
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut session = SessionState::new(
        "parent-e2e",
        agent,
        Config {
            model: "m/g".into(),
            ..Config::default()
        },
        mock,
        dir.path().to_path_buf(),
    );

    let mut events: Vec<SessionEvent> = Vec::new();
    run(&mut session, "please research".into(), |ev| events.push(ev))
        .await
        .unwrap();

    // Replay the real stream into a ChatView, sampling the parent's
    // context_used immediately before the first SubagentChild and immediately
    // after the last SubagentChild.
    let mut view = ChatView::default();
    let mut child_events = 0usize;
    for ev in &events {
        if let SessionEvent::SubagentChild { .. } = ev {
            let before = view.context_used;
            view.apply(ev);
            assert_eq!(
                view.context_used, before,
                "SubagentChild must not change parent context_used"
            );
            child_events += 1;
        } else {
            view.apply(ev);
        }
    }
    assert!(child_events > 0, "run must emit SubagentChild events");

    // The child ChatView (nested in the Subagent block) carries the child's
    // own tokens — visible when the user switches into the subagent view.
    let child_ctx = view
        .blocks
        .iter()
        .find_map(|b| match b {
            ChatBlock::Subagent { view, .. } => Some(view.context_used),
            _ => None,
        })
        .unwrap_or(0);
    assert!(
        child_ctx > 0,
        "child ChatView must track its own tokens, got {child_ctx}"
    );
}

fn two_task_turn() -> LlmEvent {
    LlmEvent::Completed {
        text: "delegating to two".into(),
        tool_calls: vec![
            CompletedToolCall {
                id: "task-A".into(),
                name: "task".into(),
                input: serde_json::json!({"prompt": "job A", "subagent_type": "explore"}),
            },
            CompletedToolCall {
                id: "task-B".into(),
                name: "task".into(),
                input: serde_json::json!({"prompt": "job B", "subagent_type": "explore"}),
            },
        ],
        usage: None,
    }
}

/// Covers the user's ORIGINAL report: "并发 subagent 之后父 context 暴涨".
/// Two task calls in one turn dispatch concurrent subagents; their interleaved
/// SubagentChild streams must not inflate the parent ChatView's context_used.
#[tokio::test]
async fn concurrent_subagents_do_not_inflate_parent_context() {
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![two_task_turn()])
            .push_script(vec![
                LlmEvent::TextDelta("child A secret output".into()),
                text_done(""),
            ])
            .push_script(vec![
                LlmEvent::TextDelta("child B secret output".into()),
                text_done(""),
            ])
            .push_script(vec![text_done("parent done")]),
    );
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut session = SessionState::new(
        "parent-conc",
        agent,
        Config {
            model: "m/g".into(),
            ..Config::default()
        },
        mock,
        dir.path().to_path_buf(),
    );

    let mut events: Vec<SessionEvent> = Vec::new();
    run(&mut session, "research two things".into(), |ev| events.push(ev))
        .await
        .unwrap();

    let subagent_count = events
        .iter()
        .filter(|e| matches!(e, SessionEvent::SubagentStart { .. }))
        .count();
    assert!(
        subagent_count >= 2,
        "precondition: must dispatch >=2 concurrent subagents, got {subagent_count}"
    );

    // Each SubagentChild must not change the parent's context_used. Assert
    // per-event: concurrent interleave means SubagentEnd (which legitimately
    // adds the finished child's summary) can land between child streams, so a
    // window span would conflate the two. Per-event isolates the contract.
    let mut view = ChatView::default();
    let mut child_events = 0usize;
    for ev in &events {
        if let SessionEvent::SubagentChild { .. } = ev {
            let before = view.context_used;
            view.apply(ev);
            assert_eq!(
                view.context_used, before,
                "SubagentChild must not change parent context_used (concurrent interleave)"
            );
            child_events += 1;
        } else {
            view.apply(ev);
        }
    }
    assert!(child_events > 0, "must see SubagentChild events");

    let child_views: Vec<&ChatView> = view
        .blocks
        .iter()
        .filter_map(|b| match b {
            ChatBlock::Subagent { view, .. } => Some(view),
            _ => None,
        })
        .collect();
    assert!(
        child_views.len() >= 2 && child_views.iter().all(|v| v.context_used > 0),
        "each of >=2 child ChatViews must track its own tokens"
    );
}
