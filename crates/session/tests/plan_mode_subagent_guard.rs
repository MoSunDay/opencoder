//! Regression test: a plan-mode agent cannot escape read-only restrictions by
//! spawning a 'build' subagent via the task tool. The runner rejects any
//! non-'explore' subagent_type in plan mode with an "Unknown subagent_type"
//! error, emits no SubagentStart, and performs no writes.
//!
//! Contracts:
//! - task tool call with subagent_type="build" from a plan agent produces a
//!   ToolEnd with is_error=true and output containing "Unknown subagent_type".
//! - No SubagentStart event is emitted.
//! - The error message must NOT advertise 'build' as a valid option (so the
//!   model is not told the escape hatch exists). Echoing the rejected type name
//!   'build' is acceptable; only the "valid options" list must omit 'build'.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionEvent, SessionState};

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

/// A plan-agent turn that tries to delegate to a 'build' subagent.
fn build_task_turn() -> LlmEvent {
    LlmEvent::Completed {
        text: "".into(),
        tool_calls: vec![CompletedToolCall {
            id: "task-build".into(),
            name: "task".into(),
            input: serde_json::json!({"prompt": "edit the file", "subagent_type": "build"}),
        }],
        usage: Some(Usage {
            input_tokens: 5,
            output_tokens: 1,
            total_tokens: 6,
        }),
    }
}

fn done_turn() -> LlmEvent {
    LlmEvent::Completed {
        text: "ok".into(),
        tool_calls: vec![],
        usage: None,
    }
}

#[tokio::test]
async fn plan_mode_blocks_build_subagent() {
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![build_task_turn()])
            .push_script(vec![done_turn()]),
    );
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("plan").unwrap();
    let mut session = SessionState::new(
        "plan-sub-guard",
        agent,
        config(),
        mock,
        dir.path().to_path_buf(),
    );

    let mut events = Vec::new();
    run(&mut session, "delegate to build".into(), |ev| {
        events.push(ev)
    })
    .await
    .unwrap();

    // The task tool call must produce a ToolEnd with is_error=true.
    let tool_end = events.iter().find(|e| {
        matches!(
            e,
            SessionEvent::ToolEnd { name, .. } if name == "task"
        )
    });
    assert!(
        tool_end.is_some(),
        "expected a ToolEnd for the task tool, got: {:?}",
        events.iter().map(ev_name).collect::<Vec<_>>()
    );
    if let SessionEvent::ToolEnd {
        is_error, output, ..
    } = tool_end.unwrap()
    {
        assert!(*is_error, "build subagent must be blocked in plan mode");
        assert!(
            output.contains("Unknown subagent_type"),
            "error must say 'Unknown subagent_type', got: {output}"
        );
        assert!(
            output.contains("build"),
            "error must echo the rejected type name, got: {output}"
        );
        // The error must NOT advertise 'build' as a valid option. It's fine
        // that the rejected type name 'build' is echoed; what matters is that
        // the "valid option" portion lists only 'explore'.
        assert!(
            !output.contains("'build' (full tools)") && !output.contains("or 'build'"),
            "error must not advertise 'build' as a valid option, got: {output}"
        );
    }

    // No SubagentStart should have been emitted — the child never ran.
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SessionEvent::SubagentStart { .. })),
        "must not start a subagent when blocked by plan-mode guard, got: {:?}",
        events.iter().map(ev_name).collect::<Vec<_>>()
    );

    // No SubagentEnd either.
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SessionEvent::SubagentEnd { .. })),
        "must not emit SubagentEnd when blocked, got: {:?}",
        events.iter().map(ev_name).collect::<Vec<_>>()
    );

    // Filesystem contract: the blocked subagent must not perform any writes.
    // The tempdir started empty; it must still be empty.
    let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert!(
        entries.is_empty(),
        "plan-mode blocked subagent must not create any files, but found: {:?}",
        entries
            .iter()
            .map(|e| e.as_ref().unwrap().path())
            .collect::<Vec<_>>()
    );
}

/// Sanity check: a plan agent CAN still spawn an 'explore' subagent (the guard
/// must not over-block the legitimate read-only path).
#[tokio::test]
async fn plan_mode_allows_explore_subagent() {
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![LlmEvent::Completed {
                text: "delegating".into(),
                tool_calls: vec![CompletedToolCall {
                    id: "task-explore".into(),
                    name: "task".into(),
                    input: serde_json::json!({"prompt": "look around", "subagent_type": "explore"}),
                }],
                usage: Some(Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    total_tokens: 15,
                }),
            }])
            .push_script(vec![done_turn()]) // child done
            .push_script(vec![done_turn()]), // parent done
    );
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("plan").unwrap();
    let mut session = SessionState::new(
        "plan-sub-allow",
        agent,
        config(),
        mock,
        dir.path().to_path_buf(),
    );

    let mut events = Vec::new();
    run(&mut session, "delegate to explore".into(), |ev| {
        events.push(ev)
    })
    .await
    .unwrap();

    assert!(
        events.iter().any(|e| {
            matches!(
                e,
                SessionEvent::SubagentStart { kind, .. } if kind == "explore"
            )
        }),
        "plan mode must allow 'explore' subagents, got: {:?}",
        events.iter().map(ev_name).collect::<Vec<_>>()
    );
}

/// Positive counterpart to `plan_mode_blocks_build_subagent`: an **act** agent
/// must be able to spawn a 'build' subagent successfully. This guards against
/// the plan-mode guard accidentally over-blocking (e.g. if the
/// `AgentKind::Plan` condition were dropped, act would also be blocked).
#[tokio::test]
async fn act_mode_allows_build_subagent() {
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![LlmEvent::Completed {
                text: "delegating to build".into(),
                tool_calls: vec![CompletedToolCall {
                    id: "task-build".into(),
                    name: "task".into(),
                    input: serde_json::json!({"prompt": "edit the file", "subagent_type": "build"}),
                }],
                usage: Some(Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    total_tokens: 15,
                }),
            }])
            .push_script(vec![done_turn()]) // child (build) done
            .push_script(vec![done_turn()]), // parent done
    );
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut session = SessionState::new(
        "act-build-allow",
        agent,
        config(),
        mock,
        dir.path().to_path_buf(),
    );

    let mut events = Vec::new();
    run(&mut session, "delegate to build".into(), |ev| {
        events.push(ev)
    })
    .await
    .unwrap();

    // The build subagent must start successfully.
    assert!(
        events.iter().any(|e| {
            matches!(
                e,
                SessionEvent::SubagentStart { kind, .. } if kind == "build"
            )
        }),
        "act mode must allow 'build' subagents, got: {:?}",
        events.iter().map(ev_name).collect::<Vec<_>>()
    );

    // And finish successfully.
    assert!(
        events
            .iter()
            .any(|e| { matches!(e, SessionEvent::SubagentEnd { ok: true, .. }) }),
        "build subagent must complete successfully (SubagentEnd ok=true), got: {:?}",
        events.iter().map(ev_name).collect::<Vec<_>>()
    );
}

fn ev_name(e: &SessionEvent) -> &'static str {
    match e {
        SessionEvent::TextDelta(_) => "TextDelta",
        SessionEvent::ToolStart { .. } => "ToolStart",
        SessionEvent::ToolEnd { .. } => "ToolEnd",
        SessionEvent::SubagentStart { .. } => "SubagentStart",
        SessionEvent::SubagentEnd { .. } => "SubagentEnd",
        SessionEvent::SubagentChild { .. } => "SubagentChild",
        SessionEvent::Done => "Done",
        SessionEvent::Error(_) => "Error",
        _ => "Other",
    }
}
