//! Pure lowering from durable messages to Responses input items.
use crate::request::{ChatRequest, RequestPurpose};
use anyhow::{anyhow, bail, Result};
use opencoder_core::{ContentBlock, Message, ProviderState, Role};
use serde_json::{json, Value};

pub fn to_body(req: &ChatRequest, scope: &ProviderState) -> Result<Value> {
    let mut body = json!({
        "model": req.model, "input": lower_input(&req.messages, scope)?,
        "stream": true, "store": false,
        "include": ["reasoning.encrypted_content"],
        "reasoning": {"summary": "auto"}
    });
    if !req.tools.is_empty() {
        body["tools"] = Value::Array(req.tools.iter().map(lower_tool).collect::<Result<_>>()?);
    }
    if let Some(choice) = &req.tool_choice {
        body["tool_choice"] = json!(choice);
    }
    if let Some(limit) = req.max_tokens {
        body["max_output_tokens"] = json!(limit);
    }
    if let Some(effort) = req
        .reasoning_effort
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        body["reasoning"]["effort"] = json!(effort);
    }
    if matches!(req.purpose, RequestPurpose::Title | RequestPurpose::Verify) {
        body["reasoning"]["effort"] = json!("low");
        body["max_output_tokens"] = json!(4096);
    }
    if let Some(key) = req
        .cache_salt
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        body["prompt_cache_key"] = json!(key);
    }
    Ok(body)
}

fn lower_tool(tool: &Value) -> Result<Value> {
    if tool["type"] != "function" {
        bail!(
            "Responses supports registered function tools, got {}",
            tool["type"]
        );
    }
    let mut function = tool
        .get("function")
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(|| anyhow!("function tool definition missing function object"))?;
    if function
        .get("name")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        bail!("function tool definition missing name");
    }
    function.insert("type".into(), json!("function"));
    function.entry("strict").or_insert(json!(false));
    Ok(Value::Object(function))
}

pub fn lower_input(messages: &[Message], scope: &ProviderState) -> Result<Vec<Value>> {
    let mut input = Vec::new();
    for msg in messages {
        if let Some(state) = msg
            .provider_state
            .as_ref()
            .filter(|s| replayable(msg, s, scope))
        {
            input.extend(state.output.iter().cloned());
            continue;
        }
        let mut content = Vec::new();
        let mut calls = Vec::new();
        for block in &msg.blocks {
            match block {
                ContentBlock::Text { text } if !text.is_empty() => content.push(json!({"type":"input_text", "text":text})),
                ContentBlock::Image { url, detail } => {
                    let mut image = json!({"type":"input_image", "image_url":url});
                    if let Some(detail) = detail { image["detail"] = json!(detail); }
                    content.push(image);
                }
                ContentBlock::ToolUse { id, name, input } => calls.push(json!({
                    "type":"function_call", "call_id":id, "name":name, "arguments":serde_json::to_string(input)?
                })),
                ContentBlock::ToolResult { tool_use_id, content, is_error, images } => {
                    let text = if *is_error { format!("[error] {content}") } else { content.clone() };
                    let output = if images.is_empty() { json!(text) } else {
                        let mut parts = vec![json!({"type":"input_text", "text":text})];
                        parts.extend(images.iter().map(|url| json!({"type":"input_image", "image_url":url})));
                        json!(parts)
                    };
                    input.push(json!({"type":"function_call_output", "call_id":tool_use_id, "output":output}));
                }
                _ => {}
            }
        }
        if !content.is_empty() {
            let role = match msg.role {
                Role::System => "system",
                Role::User | Role::Tool => "user",
                Role::Assistant => "assistant",
            };
            // Easy assistant input messages accept plain text; output_text
            // objects are replayed verbatim through provider_state above.
            let content = if msg.role == Role::Assistant {
                json!(msg.text())
            } else {
                json!(content)
            };
            input.push(json!({"role":role, "content":content}));
        }
        input.extend(calls);
    }
    Ok(input)
}

fn replayable(msg: &Message, state: &ProviderState, scope: &ProviderState) -> bool {
    if msg.role != Role::Assistant
        || state.provider != scope.provider
        || state.base_url != scope.base_url
        || state.model != scope.model
    {
        return false;
    }
    // Snapshot filtering can remove tool calls. Never resurrect a removed call
    // from opaque state (VERIFY and dangling-tool repair use such snapshots).
    let stored: Vec<_> = state
        .output
        .iter()
        .filter(|v| v["type"] == "function_call")
        .filter_map(|v| v["call_id"].as_str())
        .collect();
    let visible: Vec<_> = msg
        .blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::ToolUse { id, .. } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    stored == visible
}
