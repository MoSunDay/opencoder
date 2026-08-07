//! Integration tests for the tool-failure threshold guard.
//!
//! NOTE: the pre-existing doom-loop guard (`DOOM_THRESHOLD=20`) keys on the
//! `name:input` signature and fires on 20 identical signatures *before* tool
//! execution. To isolate the tool-failure guard, every fixture below uses a
//! distinct `input` per call (so the doom signature differs each turn) while
//! keeping the same tool *name* (so the per-name consecutive-failure counter
//! accumulates). `bash` is the workhorse for the reset test because it can both
//! fail (`exit N`, non-zero) and succeed (`echo ok`, zero).

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{tool_call::CompletedToolCall, ChatStream, LlmEvent, MockChatClient};
use opencoder_session::{run, SessionEvent, SessionState};
use serde_json::json;

/// A call to a tool that is not in the registry → always `is_error`. Each call
/// carries a unique `n` so the doom-loop `name:input` signature differs, while
/// the *name* stays constant so the consecutive-failure counter accumulates.
fn failing_tool_call(n: u32) -> LlmEvent {
    LlmEvent::Completed {
        text: String::new(),
        tool_calls: vec![CompletedToolCall {
            id: "c1".into(),
            name: "nonexistent_tool".into(),
            input: json!({ "n": n }),
        }],
        usage: None,
    }
}

/// A real `bash` call (exit code drives `is_error`). Each distinct command keeps
/// the doom-loop signature unique while the tool *name* stays `bash`.
fn bash_call(cmd: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: String::new(),
        tool_calls: vec![CompletedToolCall {
            id: "c1".into(),
            name: "bash".into(),
            input: json!({ "command": cmd }),
        }],
        usage: None,
    }
}

fn done() -> LlmEvent {
    LlmEvent::Completed {
        text: "done".into(),
        tool_calls: vec![],
        usage: None,
    }
}

/// Config with backoff disabled for fast tests.
fn fast_config() -> Config {
    let mut c = Config {
        model: "mock/test".into(),
        ..Config::default()
    };
    c.tool_guard.backoff_base_ms = 0;
    c.tool_guard.backoff_max_ms = 0;
    c
}

async fn make_session(config: Config, client: Arc<dyn ChatStream>) -> SessionState {
    // NB: must be a *persistent* path (not a dropped `TempDir`) so that real
    // tools like `bash` can `current_dir` into it when they spawn. Matches the
    // `hard_abort` integration test's pattern.
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    SessionState::new("test-session", agent, config, client, dir.keep())
}

#[tokio::test]
async fn threshold_stops_after_max_consecutive_failures() {
    // Default max_consecutive_failures = 20.
    let mut builder = MockChatClient::new();
    for n in 1..=20u32 {
        builder = builder.push_script(vec![failing_tool_call(n)]);
    }
    let mock = Arc::new(builder.push_script(vec![done()]));
    let client: Arc<dyn ChatStream> = mock.clone();
    let mut s = make_session(fast_config(), client).await;

    let result = run(&mut s, "test".into(), |_| {}).await;
    // Tool-failure now returns Err so autopilot cannot continue on an aborted
    // task. The guard still stops after 20 failures.
    assert!(
        result.is_err(),
        "tool-failure should cause run to return Err"
    );

    // Loop stopped after 20 failures — 21st script never consumed.
    assert_eq!(mock.call_count(), 20);
}

#[tokio::test]
async fn emits_error_event_on_threshold() {
    // Default max_consecutive_failures = 20.
    let mut builder = MockChatClient::new();
    for n in 1..=20u32 {
        builder = builder.push_script(vec![failing_tool_call(n)]);
    }
    let mock = Arc::new(builder);
    let client: Arc<dyn ChatStream> = mock.clone();
    let mut s = make_session(fast_config(), client).await;

    let mut errors = Vec::new();
    let result = run(&mut s, "test".into(), |ev| {
        if let SessionEvent::Error(msg) = ev {
            errors.push(msg);
        }
    })
    .await;
    // Tool-failure returns Err; the Error event was emitted via on_event first.
    assert!(
        result.is_err(),
        "tool-failure should cause run to return Err"
    );

    assert!(
        errors.iter().any(|e| e.contains("tool-failure")),
        "expected tool-failure error, got: {errors:?}"
    );
}

#[tokio::test]
async fn success_between_failures_resets_counter() {
    // All calls target the SAME tool name (`bash`); the success (`echo ok`,
    // exit 0) resets the per-name counter. Distinct commands keep the doom-loop
    // `name:input` signature unique each turn. After the reset, 20 fresh
    // failures trip the guard (default max_consecutive_failures = 20).
    let mut builder = MockChatClient::new()
        .push_script(vec![bash_call("exit 1")]) // fail 1
        .push_script(vec![bash_call("exit 2")]) // fail 2
        .push_script(vec![bash_call("echo ok")]); // success → reset
    for n in 3..=22u32 {
        // fails 1..20 after reset → trip on the 20th
        builder = builder.push_script(vec![bash_call(&format!("exit {n}"))]);
    }
    let mock = Arc::new(builder.push_script(vec![done()])); // should NOT reach
    let client: Arc<dyn ChatStream> = mock.clone();
    let mut s = make_session(fast_config(), client).await;

    let result = run(&mut s, "test".into(), |_| {}).await;
    assert!(
        result.is_err(),
        "tool-failure should cause run to return Err"
    );

    // 3 + 20 = 23 tool-call turns consumed; 24th (done) not reached.
    assert_eq!(mock.call_count(), 23);
}

#[tokio::test]
async fn disabled_guard_allows_unlimited_failures() {
    let mut cfg = fast_config();
    cfg.tool_guard.max_consecutive_failures = 0; // disable

    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![failing_tool_call(1)])
            .push_script(vec![failing_tool_call(2)])
            .push_script(vec![failing_tool_call(3)])
            .push_script(vec![failing_tool_call(4)])
            .push_script(vec![done()]),
    );
    let client: Arc<dyn ChatStream> = mock.clone();
    let mut s = make_session(cfg, client).await;

    let mut had_error = false;
    run(&mut s, "test".into(), |ev| {
        if let SessionEvent::Error(msg) = &ev {
            if msg.contains("tool-failure") {
                had_error = true;
            }
        }
    })
    .await
    .unwrap();

    // Guard disabled → no tool-failure error, loop ran to completion.
    assert!(!had_error);
    assert_eq!(mock.call_count(), 5);
}
