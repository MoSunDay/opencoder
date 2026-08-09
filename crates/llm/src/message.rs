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
    // Tool results embedded on a user message lower to `tool` messages whose
    // `content` is always a plain string (the OpenAI spec requires the `tool`
    // role to carry a string). Images a tool returned are rehomed to a fresh
    // `role:"user"` turn placed right after the matching tool result(s) so
    // vision models receive them as legal `image_url` parts — see
    // `tool_image_user_message`.
    let mut tool_images: Vec<String> = Vec::new();
    for block in &msg.blocks {
        if let ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
            images,
        } = block
        {
            out.push(tool_message(tool_use_id, content, *is_error));
            tool_images.extend_from_slice(images);
        }
    }
    if let Some(img_msg) = tool_image_user_message(&tool_images) {
        out.push(img_msg);
    }

    let text: String = msg
        .blocks
        .iter()
        .filter_map(|b| b.as_text())
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    // Multimodal path: when the message carries an Image block, emit the
    // OpenAI `content` array (`text` + `image_url`) so vision models receive
    // the attachment. Pure-text messages keep the original string `content`
    // — the `else` branch is byte-for-byte identical to the pre-image output,
    // preserving backwards compatibility and saving tokens.
    if msg.has_image() {
        let mut content: Vec<Value> = Vec::new();
        if !text.is_empty() {
            content.push(json!({ "type": "text", "text": text }));
        }
        for block in &msg.blocks {
            if let ContentBlock::Image { url, detail } = block {
                let mut image_url = serde_json::Map::new();
                image_url.insert("url".to_string(), Value::String(url.clone()));
                if let Some(d) = detail {
                    image_url.insert("detail".to_string(), Value::String(d.clone()));
                }
                content.push(json!({
                    "type": "image_url",
                    "image_url": Value::Object(image_url)
                }));
            }
        }
        if !content.is_empty() {
            out.push(json!({ "role": "user", "content": content }));
        }
    } else if !text.is_empty() {
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
        if text.is_empty() && !tool_calls.is_empty() {
            // Tool-call-only assistant turn: strict OpenAI-compatible backends
            // (some vLLM/LiteLLM/gateways) reject "content": "" on tool-use
            // history replay with HTTP 400 — emit null instead. Text-bearing
            // turns keep a string content.
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
    // Emit every `tool` message with plain-string content (spec-compliant),
    // collecting tool-returned images so they can be rehomed onto a trailing
    // `role:"user"` turn immediately after the tool results — keeping the
    // tool_call/tool_result pairing complete while delivering images to vision
    // models.
    let mut tool_images: Vec<String> = Vec::new();
    for block in &msg.blocks {
        if let ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
            images,
        } = block
        {
            out.push(tool_message(tool_use_id, content, *is_error));
            tool_images.extend_from_slice(images);
        }
    }
    if let Some(img_msg) = tool_image_user_message(&tool_images) {
        out.push(img_msg);
    }
}

/// Render a tool result's content for the model. The OpenAI `tool` role has no
/// native error flag, so an error result is prefixed with `[error]` — the
/// convention the model treats as a failed tool call. Without it the model sees
/// failure output indistinguishable from success and may repeat the failing
/// call.
fn tool_result_body(content: &str, is_error: bool) -> String {
    if is_error {
        format!("[error] {content}")
    } else {
        content.to_string()
    }
}

/// Build one OpenAI `tool` message. The `content` is **always** a plain string:
/// the OpenAI Chat Completions spec requires the `tool` role to carry a string,
/// and strict providers/proxies reject a `content` array with HTTP 400. Tool
/// images are delivered separately via [`tool_image_user_message`].
fn tool_message(tool_use_id: &str, content: &str, is_error: bool) -> Value {
    let body = tool_result_body(content, is_error);
    json!({ "role": "tool", "tool_call_id": tool_use_id, "content": body })
}

/// Build a legal `role:"user"` message carrying tool-returned images.
///
/// The OpenAI `tool` role only accepts a string `content`, so images a tool
/// returns (e.g. `view_image`) cannot ride on the `tool` message itself.
/// They are rehomed here as `image_url` parts in a fresh user turn, placed
/// immediately after the matching `tool` result(s). This keeps the
/// tool-call / tool-result pairing intact (every `tool_call_id` still has its
/// string `tool` message) while letting vision models receive the attachment
/// as a spec-compliant multimodal user request. Returns `None` when there are
/// no images so no spurious turn is emitted.
fn tool_image_user_message(images: &[String]) -> Option<Value> {
    if images.is_empty() {
        return None;
    }
    let mut parts: Vec<Value> = Vec::new();
    parts.push(json!({ "type": "text", "text": "[image returned by tool]" }));
    for url in images {
        parts.push(json!({ "type": "image_url", "image_url": { "url": url } }));
    }
    Some(json!({ "role": "user", "content": parts }))
}
