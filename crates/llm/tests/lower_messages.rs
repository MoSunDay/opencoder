//! Contract tests for `lower_messages` tool-result lowering.
//!
//! The OpenAI `tool` role carries no native error flag, so an error tool
//! result must be `[error]`-prefixed in the lowered content — otherwise the
//! model cannot tell a failed tool call from a successful one and tends to
//! repeat it. Covers both lowering paths: `Role::Tool` and `Role::User`
//! messages that embed a `ToolResult` block.

use opencoder_core::{ContentBlock, Message, Role};
use opencoder_llm::lower_messages;

fn tool_msg(id: &str, content: &str, is_error: bool) -> Message {
    Message {
        id: "m1".into(),
        role: Role::Tool,
        blocks: vec![ContentBlock::ToolResult {
            tool_use_id: id.into(),
            content: content.into(),
            is_error,
            images: Vec::new(),
        }],
        model: None,
        agent: None,
        usage: Default::default(),
        created_at: 0,
        synthetic: false,
    }
}

#[test]
fn error_tool_result_is_prefixed_in_lowering() {
    let out = lower_messages(&[tool_msg("t1", "command not found", true)]);
    assert_eq!(out.len(), 1);
    let content = out[0]["content"].as_str().unwrap();
    assert!(
        content.starts_with("[error] "),
        "error result must be [error]-prefixed, got: {content:?}"
    );
    assert!(content.contains("command not found"));
}

#[test]
fn ok_tool_result_is_not_prefixed_in_lowering() {
    let out = lower_messages(&[tool_msg("t1", "42", false)]);
    assert_eq!(out.len(), 1);
    let content = out[0]["content"].as_str().unwrap();
    assert_eq!(content, "42", "non-error result must be unchanged");
}

#[test]
fn user_role_error_tool_result_is_also_prefixed() {
    // Tool results can ride on a User message too; both lowering paths must
    // honour is_error.
    let mut m = tool_msg("t1", "permission denied", true);
    m.role = Role::User;
    let out = lower_messages(&[m]);
    let tool = out
        .iter()
        .find(|v| v["role"] == "tool")
        .expect("a tool role message must be lowered");
    let content = tool["content"].as_str().unwrap();
    assert!(
        content.starts_with("[error] "),
        "user-embedded error result must be prefixed, got: {content:?}"
    );
}

// --- multimodal (image) lowering ---

#[test]
fn pure_text_user_message_keeps_string_content() {
    // Backwards compatibility: a text-only user message must lower to a plain
    // string `content`, byte-for-byte identical to the pre-image output.
    let msg = Message::user("m1", "hello world");
    let out = lower_messages(&[msg]);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["role"], "user");
    assert!(
        out[0]["content"].is_string(),
        "pure text must stay a string"
    );
    assert_eq!(out[0]["content"].as_str().unwrap(), "hello world");
}

#[test]
fn image_user_message_lowers_to_content_array() {
    let msg = Message::user_with_images(
        "m1",
        "what is in this picture?",
        &["data:image/png;base64,iVBORw0KGgo=".to_string()],
    );
    let out = lower_messages(&[msg]);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["role"], "user");
    let content = out[0]["content"]
        .as_array()
        .expect("image msg -> content array");
    // [text, image_url]
    assert_eq!(content.len(), 2);
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[0]["text"], "what is in this picture?");
    assert_eq!(content[1]["type"], "image_url");
    assert_eq!(
        content[1]["image_url"]["url"],
        "data:image/png;base64,iVBORw0KGgo="
    );
    // detail is omitted when None (provider picks a default).
    assert!(content[1]["image_url"].get("detail").is_none());
}

#[test]
fn image_detail_is_forwarded_when_present() {
    // `user_with_images` defaults detail to None; build a block with an
    // explicit detail to confirm it reaches the lowered image_url object.
    let mut msg = Message::user_with_images("m1", "look", &[]);
    msg.blocks.push(ContentBlock::Image {
        url: "https://x/a.png".into(),
        detail: Some("low".into()),
    });
    let out = lower_messages(&[msg]);
    let content = out[0]["content"].as_array().unwrap();
    let img = content
        .iter()
        .find(|v| v["type"] == "image_url")
        .expect("image_url part present");
    assert_eq!(img["image_url"]["detail"], "low");
}

// --- tool-result images ---

#[test]
fn tool_result_with_image_lowers_to_content_array() {
    // A ToolResult carrying an image must lower to a `tool` message whose
    // `content` is an array of `text` + `image_url` parts, so vision models
    // receive the attachment.
    let msg = Message {
        id: "m1".into(),
        role: Role::Tool,
        blocks: vec![ContentBlock::ToolResult {
            tool_use_id: "t1".into(),
            content: "see image".into(),
            is_error: false,
            images: vec!["data:image/png;base64,iVBOR=".into()],
        }],
        model: None,
        agent: None,
        usage: Default::default(),
        created_at: 0,
        synthetic: false,
    };
    let out = lower_messages(&[msg]);
    assert_eq!(out.len(), 1);
    let expected = serde_json::json!({
        "role": "tool",
        "tool_call_id": "t1",
        "content": [
            { "type": "text", "text": "see image" },
            { "type": "image_url", "image_url": { "url": "data:image/png;base64,iVBOR=" } }
        ]
    });
    assert_eq!(
        out[0], expected,
        "image tool result must lower to content array"
    );
}

#[test]
fn tool_result_with_multiple_images_lowers_all_image_url_parts() {
    let msg = Message {
        id: "m1".into(),
        role: Role::Tool,
        blocks: vec![ContentBlock::ToolResult {
            tool_use_id: "t1".into(),
            content: "two shots".into(),
            is_error: false,
            images: vec![
                "data:image/png;base64,YQ==".into(),
                "https://x.test/b.jpg".into(),
            ],
        }],
        model: None,
        agent: None,
        usage: Default::default(),
        created_at: 0,
        synthetic: false,
    };
    let out = lower_messages(&[msg]);
    let content = out[0]["content"].as_array().expect("must be content array");
    // [text, image_url, image_url]
    assert_eq!(content.len(), 3, "one text + two image_url parts");
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[1]["type"], "image_url");
    assert_eq!(content[1]["image_url"]["url"], "data:image/png;base64,YQ==");
    assert_eq!(content[2]["type"], "image_url");
    assert_eq!(content[2]["image_url"]["url"], "https://x.test/b.jpg");
}

#[test]
fn error_tool_result_with_image_prefixes_text_and_keeps_image() {
    // is_error must still produce the [error] prefix on the text part while
    // the image_url part is forwarded unchanged.
    let msg = Message {
        id: "m1".into(),
        role: Role::Tool,
        blocks: vec![ContentBlock::ToolResult {
            tool_use_id: "t1".into(),
            content: "permission denied".into(),
            is_error: true,
            images: vec!["https://x.test/e.png".into()],
        }],
        model: None,
        agent: None,
        usage: Default::default(),
        created_at: 0,
        synthetic: false,
    };
    let out = lower_messages(&[msg]);
    let content = out[0]["content"].as_array().unwrap();
    assert_eq!(content[0]["type"], "text");
    assert!(
        content[0]["text"].as_str().unwrap().starts_with("[error] "),
        "error text must be prefixed"
    );
    assert_eq!(content[1]["image_url"]["url"], "https://x.test/e.png");
}

#[test]
fn text_only_tool_result_still_lowers_to_plain_string_content() {
    // Byte-for-byte backwards compat: a ToolResult with no images must keep a
    // plain-string `content`, identical to the pre-image output.
    let msg = Message {
        id: "m1".into(),
        role: Role::Tool,
        blocks: vec![ContentBlock::ToolResult {
            tool_use_id: "t1".into(),
            content: "42".into(),
            is_error: false,
            images: Vec::new(),
        }],
        model: None,
        agent: None,
        usage: Default::default(),
        created_at: 0,
        synthetic: false,
    };
    let out = lower_messages(&[msg]);
    assert_eq!(out.len(), 1);
    assert!(
        out[0]["content"].is_string(),
        "text-only tool result must stay a string content"
    );
    assert_eq!(out[0]["content"].as_str().unwrap(), "42");
    assert_eq!(out[0]["role"], "tool");
    assert_eq!(out[0]["tool_call_id"], "t1");
}

#[test]
fn user_embedded_tool_result_with_image_also_lowers_to_array() {
    // The push_user path (Role::User embedding a ToolResult) must honour the
    // image lowering just like the dedicated Role::Tool path.
    let mut msg = Message {
        id: "m1".into(),
        role: Role::User,
        blocks: vec![ContentBlock::ToolResult {
            tool_use_id: "t1".into(),
            content: "see image".into(),
            is_error: false,
            images: vec!["data:image/png;base64,iVBOR=".into()],
        }],
        model: None,
        agent: None,
        usage: Default::default(),
        created_at: 0,
        synthetic: false,
    };
    let _ = &mut msg;
    let out = lower_messages(&[msg]);
    let tool = out
        .iter()
        .find(|v| v["role"] == "tool")
        .expect("a tool role message must be lowered");
    let content = tool["content"].as_array().expect("must be content array");
    assert_eq!(content.len(), 2, "[text, image_url]");
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[1]["type"], "image_url");
    assert_eq!(
        content[1]["image_url"]["url"],
        "data:image/png;base64,iVBOR="
    );
}
