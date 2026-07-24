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
