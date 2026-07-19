//! Integration test: plan-mode bash write commands are blocked at the runner
//! level by `bash_guard`, while read-only commands execute normally.
//!
//! Contracts:
//! - A `rm -rf` call in plan mode produces a ToolEnd with is_error=true and
//!   output containing "Blocked in plan mode" — the command never executes.
//! - A `ls` call in plan mode produces a ToolEnd with is_error=false.
//! - The act agent is unaffected (no guard).

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

fn bash_turn(cmd: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: "".into(),
        tool_calls: vec![CompletedToolCall {
            id: "bash-1".into(),
            name: "bash".into(),
            input: serde_json::json!({"command": cmd}),
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
async fn plan_mode_blocks_write_command() {
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![bash_turn("rm -rf /tmp/opencoder-test-guard")])
            .push_script(vec![done_turn()]),
    );
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("plan").unwrap();
    let mut session = SessionState::new("guard-1", agent, config(), mock, dir.path().to_path_buf());

    let mut events = Vec::new();
    run(&mut session, "try to delete".into(), |ev| events.push(ev))
        .await
        .unwrap();

    let blocked = events
        .iter()
        .find(|e| matches!(e, SessionEvent::ToolEnd { name, .. } if name == "bash"));
    assert!(
        blocked.is_some(),
        "expected a ToolEnd for bash, got: {:?}",
        events.iter().map(ev_name).collect::<Vec<_>>()
    );
    if let SessionEvent::ToolEnd {
        is_error, output, ..
    } = blocked.unwrap()
    {
        assert!(*is_error, "write command must be blocked (is_error=true)");
        assert!(
            output.contains("Blocked in plan mode"),
            "output must explain the block, got: {output}"
        );
    }
}

#[tokio::test]
async fn plan_mode_allows_read_only_command() {
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![bash_turn("ls -la")])
            .push_script(vec![done_turn()]),
    );
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("plan").unwrap();
    let mut session = SessionState::new("guard-2", agent, config(), mock, dir.path().to_path_buf());

    let mut events = Vec::new();
    run(&mut session, "list files".into(), |ev| events.push(ev))
        .await
        .unwrap();

    let tool_end = events
        .iter()
        .find(|e| matches!(e, SessionEvent::ToolEnd { name, .. } if name == "bash"));
    assert!(tool_end.is_some(), "expected a ToolEnd for bash");
    if let SessionEvent::ToolEnd {
        is_error, output, ..
    } = tool_end.unwrap()
    {
        assert!(
            !*is_error,
            "read-only command must succeed, output: {output}"
        );
    }
}

#[tokio::test]
async fn act_mode_is_not_guarded() {
    // The same write command in act mode should NOT be blocked by bash_guard.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![bash_turn("mkdir -p /tmp/opencoder-test-act-guard")])
            .push_script(vec![done_turn()]),
    );
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut session = SessionState::new("guard-3", agent, config(), mock, dir.path().to_path_buf());

    let mut events = Vec::new();
    run(&mut session, "make dir".into(), |ev| events.push(ev))
        .await
        .unwrap();

    let tool_end = events
        .iter()
        .find(|e| matches!(e, SessionEvent::ToolEnd { name, .. } if name == "bash"));
    assert!(tool_end.is_some());
    if let SessionEvent::ToolEnd {
        is_error: _,
        output,
        ..
    } = tool_end.unwrap()
    {
        assert!(
            !output.contains("Blocked in plan mode"),
            "act mode must not be guarded, got: {output}"
        );
    }
}

#[tokio::test]
async fn plan_mode_allows_devnull_redirect() {
    // A read-only redirect to /dev/null (common with find/grep) must pass.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![bash_turn("find . -name '*.rs' 2>/dev/null | head")])
            .push_script(vec![done_turn()]),
    );
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("plan").unwrap();
    let mut session = SessionState::new("guard-devnull", agent, config(), mock, dir.path().to_path_buf());

    let mut events = Vec::new();
    run(&mut session, "list rust files".into(), |ev| events.push(ev))
        .await
        .unwrap();

    let tool_end = events
        .iter()
        .find(|e| matches!(e, SessionEvent::ToolEnd { name, .. } if name == "bash"));
    assert!(tool_end.is_some(), "expected a ToolEnd for bash");
    if let SessionEvent::ToolEnd {
        is_error, output, ..
    } = tool_end.unwrap()
    {
        assert!(
            !*is_error,
            "devnull redirect must succeed, output: {output}"
        );
        assert!(
            !output.contains("Blocked in plan mode"),
            "devnull redirect must not be blocked, got: {output}"
        );
    }
}

#[tokio::test]
async fn plan_mode_allows_subshell_fd_merge() {
    // `(cmd 2>&1)` and brace groups used to be blocked because the trailing
    // `)` was folded into the redirect target. These are read-only and must
    // run in plan mode.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![bash_turn("(echo hi 2>&1) | head")])
            .push_script(vec![done_turn()]),
    );
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("plan").unwrap();
    let mut session = SessionState::new(
        "guard-fdmerge",
        agent,
        config(),
        mock,
        dir.path().to_path_buf(),
    );

    let mut events = Vec::new();
    run(&mut session, "run subshell".into(), |ev| events.push(ev))
        .await
        .unwrap();

    let tool_end = events
        .iter()
        .find(|e| matches!(e, SessionEvent::ToolEnd { name, .. } if name == "bash"));
    assert!(tool_end.is_some(), "expected a ToolEnd for bash");
    if let SessionEvent::ToolEnd { is_error, output, .. } = tool_end.unwrap() {
        assert!(
            !*is_error,
            "fd-merge in subshell must succeed, output: {output}"
        );
        assert!(
            !output.contains("Blocked in plan mode"),
            "fd-merge in subshell must not be blocked, got: {output}"
        );
    }
}

#[tokio::test]
async fn plan_mode_allows_tee_to_devnull() {
    // `tee /dev/null` discards its copy and is read-only; it must not be
    // blocked in plan mode. `tee <realfile>` is still blocked (covered by the
    // unit tests in bash_guard).
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![bash_turn("echo hi | tee /dev/null")])
            .push_script(vec![done_turn()]),
    );
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("plan").unwrap();
    let mut session = SessionState::new(
        "guard-tee",
        agent,
        config(),
        mock,
        dir.path().to_path_buf(),
    );

    let mut events = Vec::new();
    run(&mut session, "tee to devnull".into(), |ev| events.push(ev))
        .await
        .unwrap();

    let tool_end = events
        .iter()
        .find(|e| matches!(e, SessionEvent::ToolEnd { name, .. } if name == "bash"));
    assert!(tool_end.is_some(), "expected a ToolEnd for bash");
    if let SessionEvent::ToolEnd { is_error, output, .. } = tool_end.unwrap() {
        assert!(
            !*is_error,
            "tee /dev/null must succeed, output: {output}"
        );
        assert!(
            !output.contains("Blocked in plan mode"),
            "tee /dev/null must not be blocked, got: {output}"
        );
    }
}

fn ev_name(e: &SessionEvent) -> &'static str {
    match e {
        SessionEvent::TextDelta(_) => "TextDelta",
        SessionEvent::ToolStart { .. } => "ToolStart",
        SessionEvent::ToolEnd { .. } => "ToolEnd",
        SessionEvent::Done => "Done",
        SessionEvent::Error(_) => "Error",
        _ => "Other",
    }
}
