//! Multimodal request integration: an image attached via `run_with_images`
//! reaches the LLM request body as an OpenAI `image_url` content part.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run_with_images, SessionState};

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

async fn session_with(client: Arc<dyn ChatStream>) -> (tempfile::TempDir, SessionState) {
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let config = Config {
        model: "qwen/qwen-vl-max".into(),
        ..Config::default()
    };
    let s = SessionState::new("img-session", agent, config, client, dir.path().to_path_buf());
    (dir, s)
}

#[tokio::test]
async fn image_attachment_reaches_request_body() {
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![LlmEvent::TextDelta("a cat".into()), done("a cat")]),
    );
    let client: Arc<dyn ChatStream> = mock.clone();
    let (_dir, mut s) = session_with(client).await;

    run_with_images(
        &mut s,
        "what is in this image?".into(),
        vec!["data:image/png;base64,iVBORw0KGgo=".to_string()],
        |_| {},
    )
    .await
    .unwrap();

    let reqs = mock.requests();
    assert!(!reqs.is_empty(), "at least one request must be captured");
    // The first lowered message is the user's multimodal turn.
    let user = reqs[0]
        .messages
        .iter()
        .find(|m| m["role"] == "user")
        .expect("a user message in the request");
    let content = user["content"]
        .as_array()
        .expect("multimodal user content must be an array");
    assert!(
        content.iter().any(|p| p["type"] == "image_url"
            && p["image_url"]["url"] == "data:image/png;base64,iVBORw0KGgo="),
        "image_url part with the data URI must be in the request body: {content:?}"
    );
    assert!(
        content.iter().any(|p| p["type"] == "text"
            && p["text"] == "what is in this image?"),
        "the text part must accompany the image"
    );
}
