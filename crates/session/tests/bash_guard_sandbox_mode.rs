//! Integration test: sandbox-mode interceptions return a model-visible error
//! so the LLM learns the session is read-only and stops retrying writes.
//!
//! Contracts:
//! - A `rm -rf` call in sandbox mode produces a ToolEnd with is_error=true
//!   and output containing "Blocked in sandbox mode" — the command never
//!   executes, the message forbids retrying, and points at `/act` (the REAL
//!   command; there is no `/agent act`) as the way out.
//! - A tool the sandbox schema never advertises (e.g. a hallucinated `edit`)
//!   is refused with the same denial and NEVER executes — no silent writes.
//! - A `ls` call in sandbox mode produces a ToolEnd with is_error=false.
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
            ..Default::default()
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
async fn sandbox_mode_blocks_write_command() {
    // NOTE: the release set is `/tmp` + `/dev/null`, so the old target
    // `/tmp/opencoder-test-guard` is now ALLOWED by policy. The guard proves
    // its blocking behavior on a cwd-relative path instead: the working
    // directory is NOT released, and if the command ever ran it would land
    // inside this test's own tempdir session dir — under test control.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![bash_turn("rm -rf ./opencoder-test-guard")])
            .push_script(vec![done_turn()]),
    );
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("sandbox").unwrap();
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
            output.contains("Blocked in sandbox mode"),
            "output must explain the block, got: {output}"
        );
        assert!(
            output.contains("`/act`") && !output.contains("/agent act"),
            "block must point at the real escape hatch (/act), got: {output}"
        );
        assert!(
            output.contains("Do not retry"),
            "denial must tell the model retries are futile, got: {output}"
        );
    }
}

#[tokio::test]
async fn sandbox_mode_allows_read_only_command() {
    // Plain read-only command, no release-set involvement: allowed anywhere.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![bash_turn("ls -la")])
            .push_script(vec![done_turn()]),
    );
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("sandbox").unwrap();
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
    // NOTE: the command is cwd-relative on purpose — /tmp is in the sandbox
    // release set, so a /tmp target would no longer distinguish guarded from
    // unguarded modes. Relative paths resolve inside this test's tempdir
    // session dir, so nothing outside the test's control is touched.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![bash_turn("mkdir -p ./opencoder-test-act-guard")])
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
            !output.contains("Blocked in sandbox mode"),
            "act mode must not be guarded, got: {output}"
        );
    }
}

#[tokio::test]
async fn sandbox_mode_allows_devnull_redirect() {
    // A read-only redirect to /dev/null (common with find/grep) must pass.
    // /dev/null is part of the declared sandbox release set (alongside /tmp),
    // so discarding output stays permitted in sandbox mode.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![bash_turn("find . -name '*.rs' 2>/dev/null | head")])
            .push_script(vec![done_turn()]),
    );
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("sandbox").unwrap();
    let mut session = SessionState::new(
        "guard-devnull",
        agent,
        config(),
        mock,
        dir.path().to_path_buf(),
    );

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
            !output.contains("Blocked in sandbox mode"),
            "devnull redirect must not be blocked, got: {output}"
        );
    }
}

#[tokio::test]
async fn sandbox_mode_allows_subshell_fd_merge() {
    // `(cmd 2>&1)` and brace groups used to be blocked because the trailing
    // `)` was folded into the redirect target. These are read-only (an fd
    // merge writes no file — no release-set involvement) and must run in
    // sandbox mode.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![bash_turn("(echo hi 2>&1) | head")])
            .push_script(vec![done_turn()]),
    );
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("sandbox").unwrap();
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
    if let SessionEvent::ToolEnd {
        is_error, output, ..
    } = tool_end.unwrap()
    {
        assert!(
            !*is_error,
            "fd-merge in subshell must succeed, output: {output}"
        );
        assert!(
            !output.contains("Blocked in sandbox mode"),
            "fd-merge in subshell must not be blocked, got: {output}"
        );
    }
}

#[tokio::test]
async fn sandbox_mode_allows_tee_to_devnull() {
    // `tee /dev/null` discards its copy and is read-only; it must not be
    // blocked in sandbox mode. /dev/null is part of the declared sandbox release
    // set; `tee <realfile>` outside /tmp + /dev/null is still blocked (covered
    // by the compat unit tests in bash_guard).
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![bash_turn("echo hi | tee /dev/null")])
            .push_script(vec![done_turn()]),
    );
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("sandbox").unwrap();
    let mut session =
        SessionState::new("guard-tee", agent, config(), mock, dir.path().to_path_buf());

    let mut events = Vec::new();
    run(&mut session, "tee to devnull".into(), |ev| events.push(ev))
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
        assert!(!*is_error, "tee /dev/null must succeed, output: {output}");
        assert!(
            !output.contains("Blocked in sandbox mode"),
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

#[tokio::test]
async fn sandbox_mode_refuses_unadvertised_tool_without_executing() {
    // `edit` is not in the sandbox allowlist, so the model is never shown it.
    // If a stale/hallucinated call still arrives it must be refused with the
    // sandbox denial — the tool body must never run (no silent writes).
    let dir = tempfile::tempdir().unwrap();
    let victim = dir.path().join("victim.txt");
    std::fs::write(&victim, "aaa").unwrap();

    let edit_turn = LlmEvent::Completed {
        text: "".into(),
        tool_calls: vec![CompletedToolCall {
            id: "edit-1".into(),
            name: "edit".into(),
            input: serde_json::json!({
                "path": victim.to_str().unwrap(),
                "old_string": "aaa",
                "new_string": "bbb",
            }),
        }],
        usage: None,
    };
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![edit_turn])
            .push_script(vec![done_turn()]),
    );
    let agent = resolve_agent("sandbox").unwrap();
    let mut session =
        SessionState::new("guard-edit", agent, config(), mock, dir.path().to_path_buf());

    let mut events = Vec::new();
    run(&mut session, "edit the file".into(), |ev| events.push(ev))
        .await
        .unwrap();

    let denied = events
        .iter()
        .find(|e| matches!(e, SessionEvent::ToolEnd { name, .. } if name == "edit"));
    assert!(
        denied.is_some(),
        "expected a ToolEnd for the refused edit, got: {:?}",
        events.iter().map(ev_name).collect::<Vec<_>>()
    );
    if let SessionEvent::ToolEnd {
        is_error, output, ..
    } = denied.unwrap()
    {
        assert!(*is_error, "unadvertised tool must be an error for the model");
        assert!(
            output.contains("Blocked in sandbox mode"),
            "denial must name the mode, got: {output}"
        );
        assert!(
            output.contains("`/act`"),
            "denial must point at the real escape hatch, got: {output}"
        );
    }
    // The decisive assertion: the edit tool never executed.
    assert_eq!(
        std::fs::read_to_string(&victim).unwrap(),
        "aaa",
        "sandbox mode must not let an unadvertised tool write"
    );
}
