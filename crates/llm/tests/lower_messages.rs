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
fn tool_result_image_rehomes_to_user_message() {
    // A ToolResult carrying an image must lower to:
    //   1. a `tool` message with plain-STRING content (OpenAI spec requires a
    //      string on the `tool` role), and
    //   2. a following `role:"user"` message whose `content` array carries the
    //      image as an `image_url` part (the only legal place for images).
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
    assert_eq!(out.len(), 2, "tool string msg + rehomed user image msg");

    let tool = &out[0];
    assert_eq!(tool["role"], "tool");
    assert_eq!(tool["tool_call_id"], "t1");
    assert!(
        tool["content"].is_string(),
        "tool content must be a plain string (spec), got: {}",
        tool["content"]
    );
    assert_eq!(tool["content"].as_str().unwrap(), "see image");

    let user = &out[1];
    assert_eq!(user["role"], "user");
    let content = user["content"]
        .as_array()
        .expect("rehomed user content must be an array");
    assert!(content
        .iter()
        .any(|p| p["type"] == "text" && p["text"] == "[image returned by tool]"));
    assert!(content.iter().any(
        |p| p["type"] == "image_url" && p["image_url"]["url"] == "data:image/png;base64,iVBOR="
    ));
}

#[test]
fn tool_result_with_multiple_images_rehomes_all() {
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
    assert_eq!(out.len(), 2);
    assert_eq!(out[0]["role"], "tool");
    assert!(out[0]["content"].is_string());
    assert_eq!(out[0]["content"].as_str().unwrap(), "two shots");

    let user = &out[1];
    assert_eq!(user["role"], "user");
    let content = user["content"].as_array().expect("user content array");
    // [text, image_url, image_url]
    assert_eq!(content.len(), 3, "one text + two image_url parts");
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[1]["type"], "image_url");
    assert_eq!(content[1]["image_url"]["url"], "data:image/png;base64,YQ==");
    assert_eq!(content[2]["type"], "image_url");
    assert_eq!(content[2]["image_url"]["url"], "https://x.test/b.jpg");
}

#[test]
fn error_tool_result_image_rehomes_with_prefixed_string() {
    // is_error must still produce the [error] prefix on the STRING tool content
    // while the image is rehomed unchanged onto the trailing user turn.
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
    assert_eq!(out.len(), 2);
    assert_eq!(out[0]["role"], "tool");
    assert!(out[0]["content"].is_string());
    assert!(
        out[0]["content"].as_str().unwrap().starts_with("[error] "),
        "error text must be prefixed"
    );
    assert!(out[0]["content"]
        .as_str()
        .unwrap()
        .contains("permission denied"));

    let user = &out[1];
    assert_eq!(user["role"], "user");
    let content = user["content"].as_array().unwrap();
    assert!(content
        .iter()
        .any(|p| p["type"] == "image_url" && p["image_url"]["url"] == "https://x.test/e.png"));
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
fn tool_message_content_is_always_string_even_with_image() {
    // Regression guard: the `tool` role content is a string regardless of
    // whether the tool returned images (strict providers HTTP-400 on an array).
    let msg = Message {
        id: "m1".into(),
        role: Role::Tool,
        blocks: vec![ContentBlock::ToolResult {
            tool_use_id: "t1".into(),
            content: "shot".into(),
            is_error: false,
            images: vec!["data:image/png;base64,AA==".into()],
        }],
        model: None,
        agent: None,
        usage: Default::default(),
        created_at: 0,
        synthetic: false,
    };
    let out = lower_messages(&[msg]);
    let tool = out
        .iter()
        .find(|v| v["role"] == "tool")
        .expect("tool message present");
    assert!(
        tool["content"].is_string(),
        "tool content must always be a string"
    );
}

#[test]
fn user_embedded_tool_result_image_rehomes_to_user_message() {
    // The push_user path (Role::User embedding a ToolResult) must rehome the
    // image just like the dedicated Role::Tool path: tool message keeps string
    // content, image lands on a trailing user turn.
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
    assert!(
        tool["content"].is_string(),
        "tool content must stay a string"
    );
    assert_eq!(tool["content"].as_str().unwrap(), "see image");
    // Exactly one user message carrying the rehomed image.
    let user_msgs: Vec<_> = out.iter().filter(|v| v["role"] == "user").collect();
    assert_eq!(user_msgs.len(), 1, "exactly one rehomed user image message");
    let content = user_msgs[0]["content"].as_array().unwrap();
    assert!(content.iter().any(
        |p| p["type"] == "image_url" && p["image_url"]["url"] == "data:image/png;base64,iVBOR="
    ));
}

// --- assistant message lowering (Fix 3: content null → empty string) ---

fn assistant_with_tool_calls(tool_id: &str) -> Message {
    Message {
        id: "m1".into(),
        role: Role::Assistant,
        blocks: vec![ContentBlock::ToolUse {
            id: tool_id.into(),
            name: "bash".into(),
            input: serde_json::json!({"cmd": "ls"}),
        }],
        model: None,
        agent: None,
        usage: Default::default(),
        created_at: 0,
        synthetic: false,
    }
}

#[test]
fn assistant_tool_only_content_is_empty_string_not_null() {
    let out = lower_messages(&[assistant_with_tool_calls("c1")]);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["role"], "assistant");
    // content must be a string (empty), NOT null — some providers reject null.
    assert!(
        out[0]["content"].is_string(),
        "assistant content must be a string, got: {:?}",
        out[0]["content"]
    );
    assert_eq!(out[0]["content"].as_str().unwrap(), "");
}

#[test]
fn assistant_text_content_is_preserved() {
    let msg = Message {
        id: "m1".into(),
        role: Role::Assistant,
        blocks: vec![ContentBlock::Text {
            text: "hello world".into(),
        }],
        model: None,
        agent: None,
        usage: Default::default(),
        created_at: 0,
        synthetic: false,
    };
    let out = lower_messages(&[msg]);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["content"].as_str().unwrap(), "hello world");
}

#[test]
fn multi_turn_tool_only_messages_all_have_string_content() {
    // Regression: multiple consecutive tool-call-only assistant turns must
    // all emit content as a string, never null.
    let msgs = vec![
        assistant_with_tool_calls("c1"),
        tool_msg("c1", "output1", false),
        assistant_with_tool_calls("c2"),
        tool_msg("c2", "output2", false),
        assistant_with_tool_calls("c3"),
        tool_msg("c3", "output3", false),
    ];
    let out = lower_messages(&msgs);
    let assistant_msgs: Vec<_> = out.iter().filter(|m| m["role"] == "assistant").collect();
    assert_eq!(assistant_msgs.len(), 3);
    for (i, m) in assistant_msgs.iter().enumerate() {
        assert!(
            m["content"].is_string(),
            "assistant #{} content must be string, got: {:?}",
            i,
            m["content"]
        );
    }
}
