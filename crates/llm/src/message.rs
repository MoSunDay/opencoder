use opencoder_core::{ContentBlock, Message, Role};
use serde_json::{json, Value};

pub type OpenAIMessage = Value;

pub fn lower_messages(messages: &[Message]) -> Vec<OpenAIMessage> {
    let mut out: Vec<OpenAIMessage> = Vec::new();
    for msg in messages {
        match msg.role {
            Role::System => push_system(&mut out, msg),
            Role::User => push_user(&mut out, msg),
            Role::Assistant => push_assistant(&mut out, msg),
            Role::Tool => push_tool_results(&mut out, msg),
        }
    }
    out
}

fn push_system(out: &mut Vec<OpenAIMessage>, msg: &Message) {
    let text: String = msg
        .blocks
        .iter()
        .filter_map(|b| b.as_text())
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    if !text.is_empty() {
        out.push(json!({ "role": "system", "content": text }));
    }
}

fn push_user(out: &mut Vec<OpenAIMessage>, msg: &Message) {
    for block in &msg.blocks {
        if let ContentBlock::ToolResult {
            tool_use_id,
            content,
            ..
        } = block
        {
            out.push(json!({ "role": "tool", "tool_call_id": tool_use_id, "content": content }));
        }
    }
    let text: String = msg
        .blocks
        .iter()
        .filter_map(|b| b.as_text())
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    if !text.is_empty() {
        out.push(json!({ "role": "user", "content": text }));
    }
}

fn push_assistant(out: &mut Vec<OpenAIMessage>, msg: &Message) {
    let text: String = msg
        .blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let reasoning: String = msg
        .blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Reasoning { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");
    let tool_calls: Vec<Value> = msg
        .blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::ToolUse { id, name, input } => Some(json!({
                "id": id,
                "type": "function",
                "function": { "name": name, "arguments": serde_json::to_string(input).unwrap_or_else(|_| "{}".into()) }
            })),
            _ => None,
        })
        .collect();

    if text.is_empty() && tool_calls.is_empty() && reasoning.is_empty() {
        return;
    }
    let mut m = serde_json::Map::new();
    m.insert("role".to_string(), Value::String("assistant".into()));
    m.insert(
        "content".to_string(),
        if text.is_empty() {
            Value::Null
        } else {
            Value::String(text)
        },
    );
    if !tool_calls.is_empty() {
        m.insert("tool_calls".to_string(), Value::Array(tool_calls));
    }
    if !reasoning.is_empty() {
        m.insert("reasoning_content".to_string(), Value::String(reasoning));
    }
    out.push(Value::Object(m));
}

fn push_tool_results(out: &mut Vec<OpenAIMessage>, msg: &Message) {
    for block in &msg.blocks {
        if let ContentBlock::ToolResult {
            tool_use_id,
            content,
            ..
        } = block
        {
            out.push(json!({ "role": "tool", "tool_call_id": tool_use_id, "content": content }));
        }
    }
}
