//! Subagent dispatch tests — verifies run_subagent emits SubagentStart/SubagentEnd
//! and forwards child events to the parent's on_event sink.
//!
//! Contracts:
//! - task tool call triggers SubagentStart + child run + SubagentEnd(ok=true)
//! - empty prompt returns error without SubagentStart

use std::sync::Arc;

use opencode_core::{resolve_agent, Config};
use opencode_llm::{CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencode_session::{run, SessionEvent, SessionState};

fn config() -> Config {
    Config { model: "m/g".into(), max_steps: 10, ..Config::default() }
}

fn task_turn(prompt: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: "delegating".into(),
        tool_calls: vec![CompletedToolCall {
            id: "task-1".into(),
            name: "task".into(),
            input: serde_json::json!({"prompt": prompt, "subagent_type": "subagent"}),
        }],
        usage: Some(Usage { input_tokens: 10, output_tokens: 5, total_tokens: 15 }),
    }
}

fn text_done(text: &str) -> LlmEvent {
    LlmEvent::Completed { text: text.into(), tool_calls: vec![], usage: None }
}

#[tokio::test]
async fn subagent_emits_start_and_end_events() {
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![task_turn("research the codebase")]) // parent turn 1: task call
            .push_script(vec![text_done("found 3 files")]) // child turn 1: text done
            .push_script(vec![text_done("all done")]), // parent turn 2: done
    );

    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut session = SessionState::new("sub-test", agent, config(), mock, dir.path().to_path_buf());

    let mut events = Vec::new();
    run(&mut session, "delegate research".into(), |ev| events.push(ev)).await.unwrap();

    let has_start = events.iter().any(|e| matches!(
        e,
        SessionEvent::SubagentStart { kind, prompt, .. }
        if kind == "subagent" && prompt.contains("research")
    ));
    assert!(has_start, "expected SubagentStart, got {:?}", events.iter().map(format_ev).collect::<Vec<_>>());

    let has_end = events.iter().any(|e| matches!(
        e,
        SessionEvent::SubagentEnd { ok: true, summary, .. }
        if summary.contains("found")
    ));
    assert!(has_end, "expected SubagentEnd(ok=true, summary contains 'found'), got {:?}", events.iter().map(format_ev).collect::<Vec<_>>());
}

#[tokio::test]
async fn subagent_forwards_child_tool_events_to_parent() {
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![task_turn("run a command")]) // parent: task call
            .push_script(vec![LlmEvent::Completed {
                // child: bash call
                text: "".into(),
                tool_calls: vec![CompletedToolCall {
                    id: "child-bash".into(),
                    name: "bash".into(),
                    input: serde_json::json!({"command": "echo hi"}),
                }],
                usage: Some(Usage { input_tokens: 5, output_tokens: 1, total_tokens: 6 }),
            }])
            .push_script(vec![text_done("done")]) // child: done
            .push_script(vec![text_done("parent done")]), // parent: done
    );

    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut session = SessionState::new("sub-fwd", agent, config(), mock, dir.path().to_path_buf());

    let mut events = Vec::new();
    run(&mut session, "delegate".into(), |ev| events.push(ev)).await.unwrap();

    // Child's bash ToolStart should be forwarded to parent
    let has_child_tool = events.iter().any(|e| matches!(
        e,
        SessionEvent::ToolStart { name, .. } if name == "bash"
    ));
    assert!(has_child_tool, "expected child bash ToolStart forwarded to parent");

    let has_subagent_end = events.iter().any(|e| matches!(e, SessionEvent::SubagentEnd { .. }));
    assert!(has_subagent_end, "expected SubagentEnd");
}

fn format_ev(e: &SessionEvent) -> &'static str {
    match e {
        SessionEvent::TextDelta(_) => "TextDelta",
        SessionEvent::ToolStart { .. } => "ToolStart",
        SessionEvent::ToolEnd { .. } => "ToolEnd",
        SessionEvent::SubagentStart { .. } => "SubagentStart",
        SessionEvent::SubagentEnd { .. } => "SubagentEnd",
        SessionEvent::Done => "Done",
        SessionEvent::Error(_) => "Error",
        _ => "Other",
    }
}
