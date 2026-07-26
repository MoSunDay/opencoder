//! Contract test: a tool that returns images (e.g. `view_image`) must cause
//! the subsequent LLM request to rehome those images as `image_url` parts on a
//! `role:"user"` message (the `tool` role carries a plain string per the OpenAI
//! spec). This is the acceptance contract for multimodal TUI support under the
//! qwen vision-model namespace — deterministic, zero-network.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionState};

/// 1×1 transparent PNG (70 bytes). Written to the working dir so the
/// `view_image` tool can read it.
const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15,
    0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xfc, 0xff,
    0x9f, 0xa1, 0x1e, 0x00, 0x07, 0x82, 0x02, 0x7f, 0x3d, 0xc8, 0x48, 0xef, 0x00, 0x00, 0x00,
    0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

fn done(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.to_string(),
        tool_calls: Vec::<CompletedToolCall>::new(),
        usage: Some(Usage {
            input_tokens: 1,
            output_tokens: 1,
            total_tokens: 2,
            ..Default::default()
        }),
    }
}

/// An LLM turn that asks the model to call `view_image` on a relative path.
fn view_image_turn() -> LlmEvent {
    LlmEvent::Completed {
        text: "Let me look at that image.".into(),
        tool_calls: vec![CompletedToolCall {
            id: "vi-1".into(),
            name: "view_image".into(),
            input: serde_json::json!({"path": "tiny.png"}),
        }],
        usage: None,
    }
}

/// Build a session configured for a vision-capable model under the qwen
/// provider namespace, mirroring a production qwen3-vl deployment.
async fn qwen_session(client: Arc<dyn ChatStream>) -> (tempfile::TempDir, SessionState) {
    let dir = tempfile::tempdir().unwrap();
    // Write the test image to the working dir so view_image can read it.
    std::fs::write(dir.path().join("tiny.png"), TINY_PNG).unwrap();
    let agent = resolve_agent("act").unwrap();
    let config = Config {
        model: "qwen3.8/qwen3-vl-plus".into(),
        ..Config::default()
    };
    let s = SessionState::new(
        "contract-session",
        agent,
        config,
        client,
        dir.path().to_path_buf(),
    );
    (dir, s)
}

#[tokio::test]
async fn tool_returned_image_reaches_request_body() {
    // Script: turn 1 → view_image tool call; turn 2 → final text.
    let mock = Arc::new(
        MockChatClient::new().push_script(vec![LlmEvent::TextDelta("looking".into()), view_image_turn()])
            .push_script(vec![LlmEvent::TextDelta("it is a 1x1 image".into()), done("it is a 1x1 image")]),
    );
    let client: Arc<dyn ChatStream> = mock.clone();
    let (_dir, mut s) = qwen_session(client).await;

    run(&mut s, "analyze tiny.png".into(), |_| {}).await.unwrap();

    let reqs = mock.requests();
    assert!(
        reqs.len() >= 2,
        "expected at least 2 requests (tool call + result), got {}",
        reqs.len()
    );

    // The second request must carry the tool result as a STRING-content `tool`
    // message (OpenAI spec) ...
    let second = &reqs[1];
    let tool_msg = second
        .messages
        .iter()
        .find(|m| m["role"] == "tool")
        .unwrap_or_else(|| panic!("expected a 'tool' message in second request: {:?}", second.messages));
    assert!(
        tool_msg["content"].is_string(),
        "tool content must be a plain string (OpenAI spec), got: {tool_msg:?}"
    );

    // ... and the tool-returned image is rehomed onto a `role:"user"` message as
    // a legal `image_url` part (images cannot live on the `tool` role).
    let user_with_image = second
        .messages
        .iter()
        .filter(|m| m["role"] == "user")
        .find(|m| {
            m["content"]
                .as_array()
                .map(|parts| parts.iter().any(|p| p["type"] == "image_url"))
                .unwrap_or(false)
        })
        .unwrap_or_else(|| panic!("expected a 'user' message with an image_url part: {:?}", second.messages));

    let content = user_with_image["content"]
        .as_array()
        .expect("user image content must be an array");
    assert!(
        content.iter().any(|p| p["type"] == "image_url"
            && p["image_url"]["url"]
                .as_str()
                .map(|u| u.starts_with("data:image/"))
                .unwrap_or(false)),
        "user message must contain an image_url part with a data URI: {content:?}"
    );
    assert!(
        content.iter().any(|p| p["type"] == "text"),
        "user message must also carry a text part: {content:?}"
    );
}
