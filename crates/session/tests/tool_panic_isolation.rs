//! Integration test for the tool-panic isolation fix in `runner/mod.rs`.
//!
//! Contract: a panic inside a tool's `execute` must NOT propagate out of the
//! `FuturesUnordered` in `run_with_registry` and abort the whole `run_loop`
//! (which would strand in-flight subagent futures and leave DB rows in
//! `Running`). Instead the panic is caught by
//! `AssertUnwindSafe(...).catch_unwind()` and converted into an error
//! `ToolOutput` (`is_error: true`, content "tool `<name>` panicked: ..."),
//! surfacing as a `SessionEvent::ToolEnd { is_error: true, .. }`. The session
//! then continues to the next turn normally.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use opencoder_core::{resolve_agent, Config, ContentBlock, Tool, ToolArc, ToolContext, ToolOutput};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::runner::run_with_registry;
use opencoder_session::{SessionEvent, SessionState};
use serde_json::{json, Value};

/// A mock tool whose `execute` always panics, simulating a buggy tool
/// implementation. Unit struct, no encapsulated state.
struct BoomTool;

#[async_trait]
impl Tool for BoomTool {
    fn name(&self) -> &str {
        "boom"
    }
    fn description(&self) -> &str {
        "always panics inside execute"
    }
    fn parameters(&self) -> Value {
        json!({})
    }
    async fn execute(&self, _input: Value, _ctx: &ToolContext) -> anyhow::Result<ToolOutput> {
        panic!("boom exploded")
    }
}

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

fn usage() -> Usage {
    Usage {
        input_tokens: 5,
        output_tokens: 5,
        total_tokens: 10,
        ..Default::default()
    }
}

/// Registry containing ONLY the panicking tool.
fn boom_registry() -> std::collections::HashMap<String, ToolArc> {
    let mut m = std::collections::HashMap::new();
    m.insert("boom".to_string(), Arc::new(BoomTool) as ToolArc);
    m
}

#[tokio::test]
async fn panicking_tool_does_not_crash_run_loop() {
    // Round 1: request the panicking tool. Round 2: idle text, no tool calls.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![LlmEvent::Completed {
                text: "".into(),
                tool_calls: vec![CompletedToolCall {
                    id: "boom-1".into(),
                    name: "boom".into(),
                    input: json!({}),
                }],
                usage: Some(usage()),
            }])
            .push_script(vec![LlmEvent::Completed {
                text: "all done".into(),
                tool_calls: vec![],
                usage: Some(usage()),
            }]),
    );

    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "panic-iso",
        resolve_agent("act").unwrap(),
        config(),
        mock.clone() as Arc<dyn ChatStream>,
        dir.path().to_path_buf(),
    );

    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    let reg = boom_registry();

    let result = run_with_registry(
        &mut session,
        "run the boom tool".into(),
        Vec::new(),
        &reg,
        move |ev: SessionEvent| ev_clone.lock().unwrap().push(ev),
    )
    .await;

    // The run must NOT be aborted by the tool panic: it returns Ok(()).
    assert!(
        result.is_ok(),
        "run_with_registry should survive a tool panic, got Err: {:?}",
        result.err()
    );

    let evs = events.lock().unwrap();

    // The panic surfaced as an error ToolEnd for "boom".
    let boom_err = evs.iter().any(|e| {
        matches!(
            e,
            SessionEvent::ToolEnd {
                name,
                is_error: true,
                output,
                ..
            } if name == "boom" && output.contains("panicked")
        )
    });
    assert!(
        boom_err,
        "expected ToolEnd(boom, is_error=true, output contains 'panicked'); events: {:?}",
        evs
    );

    // The session continued past the panic: it reached idle (Done), the LLM
    // was called twice (round 1 tool + round 2 text), and the round-2 text
    // "all done" was persisted into the assistant message transcript.
    assert!(
        evs.iter().any(|e| matches!(e, SessionEvent::Done)),
        "expected Done event after continuing past the panic; events: {:?}",
        evs
    );
    assert_eq!(
        mock.call_count(),
        2,
        "expected two LLM rounds (panic turn + continuation turn)"
    );

    let saw_text = session.messages.iter().any(|m| {
        m.blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::Text { text } if text == "all done"))
    });
    assert!(
        saw_text,
        "expected the round-2 assistant message to contain 'all done'; messages: {:?}",
        session.messages
    );
}
