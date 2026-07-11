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
    let mut before_first_child: Option<u64> = None;
    let mut after_last_child: Option<u64> = None;
    for ev in &events {
        if matches!(ev, SessionEvent::SubagentChild { .. }) && before_first_child.is_none() {
            before_first_child = Some(view.context_used);
        }
        view.apply(ev);
        if matches!(ev, SessionEvent::SubagentChild { .. }) {
            after_last_child = Some(view.context_used);
        }
    }

    let before = before_first_child.expect("run must emit SubagentChild events");
    let after = after_last_child.expect("run must emit SubagentChild events");
    assert_eq!(
        before, after,
        "parent context_used must not grow while subagent child events stream in"
    );

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
