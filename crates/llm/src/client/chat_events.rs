use super::usage::parse_usage;
use crate::{
    event::{LlmEvent, Usage},
    tool_call::ToolAccumulator,
};
use anyhow::{anyhow, Result};
use serde_json::Value;
use tokio::sync::mpsc;

pub(super) async fn handle_event(
    parsed: &Value,
    tools: &mut ToolAccumulator,
    usage: &mut Option<Usage>,
    finished: &mut bool,
    text_buf: &mut String,
    streamed_reasoning: &mut bool,
    tx: &mpsc::Sender<LlmEvent>,
) -> Result<()> {
    if let Some(u) = parsed.get("usage") {
        *usage = Some(parse_usage(u));
    }
    let choices = match parsed.get("choices").and_then(|c| c.as_array()) {
        Some(c) => c,
        None => return Ok(()),
    };
    for choice in choices {
        if let Some(delta) = choice.get("delta") {
            if emit_delta(delta, tools, text_buf, tx).await? {
                *streamed_reasoning = true;
            }
        }
        if let Some(fr) = choice.get("finish_reason").and_then(|v| v.as_str()) {
            if matches!(fr, "length" | "content_filter") {
                return Err(anyhow!("chat response incomplete: {fr}"));
            }
            *finished = true;
        }
        // Non-streaming fallback: some providers (notably at max/xhigh
        // reasoning effort) deliver the full reasoning only on the last frame
        // under `choice.message.reasoning_content` (and aliases / structured
        // `content` blocks) rather than as per-delta `delta.reasoning_content`.
        // Emit it as a single ReasoningDelta so the Thinking label still shows.
        //
        // Cross-frame guard: only fire when NO reasoning has been streamed as
        // deltas earlier in this turn. Providers that both stream reasoning via
        // `delta.reasoning_content` AND repeat it wholesale in the final
        // `choice.message` would otherwise double-emit — duplicating the UI
        // Thinking block and, on tool turns, double-persisting the reasoning
        // that is resent to the API.
        if !*streamed_reasoning {
            if let Some(msg) = choice.get("message") {
                let reasoning = extract_reasoning(msg);
                if let Some(r) = reasoning {
                    if !r.is_empty() {
                        *streamed_reasoning = true;
                        let _ = tx.send(LlmEvent::ReasoningDelta(r)).await;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Extract reasoning text from a `delta` or `message` object, accepting the
/// many provider-specific shapes seen at higher reasoning effort levels
/// (max / xhigh / high):
///
/// 1. A plain-string field under any of the alias keys below.
/// 2. A JSON array under one of those keys, whose string elements are joined.
/// 3. A structured `content` array with `{type: "thinking"|"reasoning", ...}`
///    blocks (OpenAI-style reasoning content blocks; the text lives under
///    `text` or `content`).
///
/// Returns `None` when no reasoning is present. Aliases are checked in
/// priority order; the first non-empty match wins.
pub(super) fn extract_reasoning(obj: &Value) -> Option<String> {
    // 1 + 2: string or array-of-strings under an alias key.
    const REASONING_KEYS: &[&str] = &[
        "reasoning_content",
        "reasoning",
        "thinking",
        "reasoning_summary",
        "chain_of_thought",
        "analysis",
        "thoughts",
    ];
    for key in REASONING_KEYS {
        if let Some(v) = obj.get(key) {
            if let Some(s) = v.as_str() {
                if !s.is_empty() {
                    return Some(s.to_string());
                }
            } else if let Some(arr) = v.as_array() {
                let mut joined = String::new();
                for item in arr {
                    if let Some(s) = item.as_str() {
                        joined.push_str(s);
                    }
                }
                if !joined.is_empty() {
                    return Some(joined);
                }
            }
        }
    }
    // 3: structured `content` array with thinking/reasoning blocks.
    if let Some(content) = obj.get("content").and_then(|v| v.as_array()) {
        let mut joined = String::new();
        for item in content {
            let is_reasoning = matches!(
                item.get("type").and_then(|v| v.as_str()),
                Some("thinking") | Some("reasoning")
            );
            if !is_reasoning {
                continue;
            }
            let text = item
                .get("text")
                .and_then(|v| v.as_str())
                .or_else(|| item.get("content").and_then(|v| v.as_str()))
                .unwrap_or("");
            joined.push_str(text);
        }
        if !joined.is_empty() {
            return Some(joined);
        }
    }
    None
}

pub(super) async fn emit_delta(
    delta: &Value,
    tools: &mut ToolAccumulator,
    text_buf: &mut String,
    tx: &mpsc::Sender<LlmEvent>,
) -> Result<bool> {
    // Whether this frame carried reasoning. The caller uses it as a cross-frame
    // guard so the non-streaming fallback doesn't re-emit delivered reasoning.
    let mut emitted_reasoning = false;
    // Alias-key reasoning is checked regardless of content shape: some
    // providers send a content array AND reasoning via alias keys in one delta.
    if let Some(reasoning) = extract_reasoning(delta) {
        if !reasoning.is_empty() {
            emitted_reasoning = true;
            let _ = tx.send(LlmEvent::ReasoningDelta(reasoning)).await;
        }
    }

    // Structured content array (text/thinking blocks), iterated in order.
    if let Some(content) = delta.get("content").and_then(|v| v.as_array()) {
        for item in content {
            match item.get("type").and_then(|v| v.as_str()) {
                Some("text") => {
                    let t = item
                        .get("text")
                        .and_then(|v| v.as_str())
                        .or_else(|| item.get("content").and_then(|v| v.as_str()));
                    if let Some(t) = t {
                        if !t.is_empty() {
                            text_buf.push_str(t);
                            let _ = tx.send(LlmEvent::TextDelta(t.to_string())).await;
                        }
                    }
                }
                Some("thinking") | Some("reasoning") if !emitted_reasoning => {
                    let t = item
                        .get("text")
                        .and_then(|v| v.as_str())
                        .or_else(|| item.get("content").and_then(|v| v.as_str()))
                        .unwrap_or("");
                    if !t.is_empty() {
                        emitted_reasoning = true;
                        let _ = tx.send(LlmEvent::ReasoningDelta(t.to_string())).await;
                    }
                }
                _ => {}
            }
        }
    } else {
        // Flat OpenAI-compatible deltas may carry the final answer token.
        if let Some(content) = delta.get("content").and_then(|v| v.as_str()) {
            if !content.is_empty() {
                text_buf.push_str(content);
                let _ = tx.send(LlmEvent::TextDelta(content.to_string())).await;
            }
        }
    }
    if let Some(tool_calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
        for tc in tool_calls {
            let index = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let id = tc.get("id").and_then(|v| v.as_str());
            let name = tc
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str());
            let args = tc
                .get("function")
                .and_then(|f| f.get("arguments"))
                .and_then(|v| v.as_str());
            for ev in tools.apply(index, id, name, args) {
                let _ = tx.send(ev).await;
            }
        }
    }
    Ok(emitted_reasoning)
}
