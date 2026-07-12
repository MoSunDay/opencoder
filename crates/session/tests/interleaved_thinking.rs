//! Tests for interleaved thinking: reasoning_content is captured from the
//! stream and persisted into the assistant message on tool-call turns, then
//! sent back on the next request via push_assistant's reasoning_content field.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, ContentBlock, Message, Role};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionState};

fn reasoning_delta(text: &str) -> LlmEvent {
    LlmEvent::ReasoningDelta(text.to_string())
}

fn text_delta(text: &str) -> LlmEvent {
    LlmEvent::TextDelta(text.to_string())
}

fn completed_with_tools(text: &str, tool_calls: Vec<CompletedToolCall>) -> LlmEvent {
    LlmEvent::Completed {
        text: text.to_string(),
        tool_calls,
        usage: Some(Usage {
            input_tokens: 10,
            output_tokens: 20,
            total_tokens: 30,
        }),
    }
}

fn completed_no_tools(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.to_string(),
        tool_calls: Vec::new(),
        usage: Some(Usage {
            input_tokens: 10,
            output_tokens: 20,
            total_tokens: 30,
        }),
    }
}

fn base_config() -> Config {
    Config {
        model: "main/glm-5.2".into(),
        ..Config::default()
    }
}

async fn session_with(
    config: Config,
    client: Arc<dyn ChatStream>,
) -> (tempfile::TempDir, SessionState) {
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let s = SessionState::new(
        "test-session",
        agent,
        config,
        client,
        dir.path().to_path_buf(),
    );
    (dir, s)
}

/// A tool call that the mock will emit; the "bash" tool is in the default
/// registry and will be executed (yielding a result the model sees next turn).
fn bash_tool_call() -> CompletedToolCall {
    CompletedToolCall {
        id: "call_1".into(),
        name: "bash".into(),
        input: serde_json::json!({ "command": "echo hello" }),
    }
}

#[tokio::test]
async fn reasoning_persisted_on_tool_call_turn() {
    // Turn 1: reasoning + tool_call → must persist Reasoning block.
    // Turn 2: plain text → loop ends.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![
                reasoning_delta("Let me think..."),
                completed_with_tools("", vec![bash_tool_call()]),
            ])
            .push_script(vec![completed_no_tools("done")]),
    );
    let client: Arc<dyn ChatStream> = mock.clone();
    let (_dir, mut s) = session_with(base_config(), client).await;

    run(&mut s, "run echo".into(), |_| {}).await.unwrap();

    // The assistant message from turn 1 must contain a Reasoning block.
    let assistant_msgs: Vec<&Message> = s
        .messages
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .collect();
    assert!(assistant_msgs.len() >= 2, "need >=2 assistant turns");
    let first = assistant_msgs[0];
    let has_reasoning = first
        .blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Reasoning { .. }));
    assert!(
        has_reasoning,
        "tool-call turn must persist a Reasoning block, blocks: {:?}",
        first
            .blocks
            .iter()
            .map(|b| match b {
                ContentBlock::Text { .. } => "Text",
                ContentBlock::Reasoning { .. } => "Reasoning",
                ContentBlock::ToolUse { name, .. } => name,
                ContentBlock::ToolResult { .. } => "ToolResult",
            })
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn reasoning_not_persisted_on_non_tool_turn() {
    // Single turn with reasoning + text but NO tool calls → no Reasoning block.
    let mock = Arc::new(MockChatClient::new().push_script(vec![
        reasoning_delta("Hmm..."),
        text_delta("answer"),
        completed_no_tools("answer"),
    ]));
    let client: Arc<dyn ChatStream> = mock.clone();
    let (_dir, mut s) = session_with(base_config(), client).await;

    run(&mut s, "hi".into(), |_| {}).await.unwrap();

    let assistant_msgs: Vec<&Message> = s
        .messages
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .collect();
    assert!(!assistant_msgs.is_empty());
    let has_reasoning = assistant_msgs[0]
        .blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Reasoning { .. }));
    assert!(
        !has_reasoning,
        "non-tool turn must NOT persist a Reasoning block"
    );
}

#[tokio::test]
async fn interleaved_disabled_skips_reasoning_persistence() {
    // With interleaved_thinking=false, even a tool-call turn must not persist
    // reasoning.
    let mut config = base_config();
    config.interleaved_thinking = Some(false);

    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![
                reasoning_delta("thinking..."),
                completed_with_tools("", vec![bash_tool_call()]),
            ])
            .push_script(vec![completed_no_tools("done")]),
    );
    let client: Arc<dyn ChatStream> = mock.clone();
    let (_dir, mut s) = session_with(config, client).await;

    run(&mut s, "run echo".into(), |_| {}).await.unwrap();

    let assistant_msgs: Vec<&Message> = s
        .messages
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .collect();
    assert!(!assistant_msgs.is_empty());
    let has_reasoning = assistant_msgs[0]
        .blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Reasoning { .. }));
    assert!(
        !has_reasoning,
        "interleaved_thinking=false must NOT persist reasoning even on tool-call turns"
    );
}

#[tokio::test]
async fn reasoning_sent_back_in_second_request_body() {
    // Turn 1: reasoning + tool_call → Reasoning block persisted.
    // Turn 2: the second request body must contain reasoning_content in the
    // assistant message (proving push_assistant serializes it).
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![
                reasoning_delta("I need to run a command."),
                completed_with_tools("", vec![bash_tool_call()]),
            ])
            .push_script(vec![completed_no_tools("all done")]),
    );
    let client: Arc<dyn ChatStream> = mock.clone();
    let (_dir, mut s) = session_with(base_config(), client).await;

    run(&mut s, "run echo".into(), |_| {}).await.unwrap();

    let reqs = mock.requests();
    assert!(reqs.len() >= 2, "need >=2 requests (turn1 + turn2)");
    // The second request's messages must include an assistant message with
    // reasoning_content.
    let second = &reqs[1];
    let assistant_msgs: Vec<&serde_json::Value> = second
        .messages
        .iter()
        .filter(|m| m.get("role").and_then(|v| v.as_str()) == Some("assistant"))
        .collect();
    let any_has_reasoning = assistant_msgs
        .iter()
        .any(|m| m.get("reasoning_content").is_some());
    assert!(
        any_has_reasoning,
        "second request must include reasoning_content in an assistant message"
    );
}
