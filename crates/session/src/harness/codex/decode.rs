//! JSONL reducer: source state + one line -> new state + normalized effects.
use crate::SessionEvent;
use anyhow::{bail, ensure, Context, Result};
use opencoder_core::{ContentBlock, Message, MessageUsage, Role};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Default)]
pub struct Decoder {
    pub prefix: String,
    pub thread: Option<String>,
    pub started: bool,
    pub completed: bool,
    pub failed: Option<String>,
    pub usage: MessageUsage,
    items: BTreeMap<String, ItemState>,
}
#[derive(Clone, Default)]
struct ItemState {
    text: String,
    started: bool,
    done: bool,
    kind: String,
}
#[derive(Default)]
pub struct Projection {
    pub events: Vec<SessionEvent>,
    pub messages: Vec<Message>,
}

fn message(id: String, role: Role, blocks: Vec<ContentBlock>) -> Message {
    Message {
        provider_state: None,
        id,
        role,
        blocks,
        model: None,
        agent: None,
        usage: Default::default(),
        created_at: opencoder_core::message::now_ms(),
        synthetic: false,
        display: None,
    }
}

pub fn decode(mut state: Decoder, line: &str) -> Result<(Decoder, Projection)> {
    let value: Value = serde_json::from_str(line).context("invalid Codex JSONL")?;
    let kind = value["type"].as_str().context("Codex event missing type")?;
    let mut out = Projection::default();
    match kind {
        "thread.started" => {
            let id = value["thread_id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .context("Codex thread ID missing")?;
            if let Some(previous) = &state.thread {
                ensure!(previous == id, "Codex thread changed during turn");
            }
            state.thread = Some(id.into());
        }
        "turn.started" => {
            ensure!(
                !state.started && !state.completed,
                "duplicate Codex turn start"
            );
            state.started = true;
            out.events.push(SessionEvent::LlmRoundStart {
                started_at_ms: opencoder_core::message::now_ms(),
            });
        }
        "turn.completed" => {
            ensure!(
                state.started && !state.completed,
                "unexpected Codex turn completion"
            );
            ensure!(
                !state.items.values().any(|i| !i.done),
                "Codex completed with unfinished items"
            );
            let usage = &value["usage"];
            let input = usage["input_tokens"]
                .as_u64()
                .context("Codex input usage missing")?;
            let output = usage["output_tokens"]
                .as_u64()
                .context("Codex output usage missing")?;
            state.usage = MessageUsage {
                reasoning_tokens: 0,
                input_tokens: input,
                output_tokens: output,
                total_tokens: input.saturating_add(output),
                cache_read_tokens: usage["cached_input_tokens"].as_u64().unwrap_or(0),
                cache_creation_tokens: usage["cache_write_input_tokens"].as_u64().unwrap_or(0),
            };
            state.completed = true;
            out.events.push(SessionEvent::LlmUsage {
                total_tokens: state.usage.total_tokens,
                input_tokens: input,
                output_tokens: output,
            });
            out.events.push(SessionEvent::LlmRoundEnd);
        }
        "turn.failed" | "error" => {
            let error = if kind == "turn.failed" {
                &value["error"]["message"]
            } else {
                &value["message"]
            };
            let error = error
                .as_str()
                .context("Codex error missing message")?
                .to_owned();
            state.failed = Some(error.clone());
            out.events.push(SessionEvent::Error(error));
        }
        "item.started" | "item.updated" | "item.completed" => {
            ensure!(
                state.started && !state.completed,
                "Codex item outside active turn"
            );
            let item = &value["item"];
            let source_id = item["id"].as_str().context("Codex item missing ID")?;
            let id = format!("{}:{source_id}", state.prefix);
            let item_kind = item["type"].as_str().context("Codex item missing type")?;
            let entry = state.items.entry(source_id.into()).or_default();
            if !entry.kind.is_empty() {
                ensure!(entry.kind == item_kind, "Codex item type changed");
            }
            entry.kind = item_kind.into();
            if entry.done {
                return Ok((state, out));
            }
            let done = kind == "item.completed";
            match item_kind {
                "agent_message" | "reasoning" => {
                    let text = item["text"]
                        .as_str()
                        .context("Codex text item missing text")?;
                    let delta = text
                        .strip_prefix(&entry.text)
                        .context("Codex text update is not cumulative")?;
                    if !delta.is_empty() {
                        out.events.push(if item_kind == "reasoning" {
                            SessionEvent::ReasoningDelta(delta.into())
                        } else {
                            SessionEvent::TextDelta(delta.into())
                        });
                    }
                    entry.text = text.into();
                    if done {
                        let block = if item_kind == "reasoning" {
                            ContentBlock::Reasoning { text: text.into() }
                        } else {
                            ContentBlock::text(text)
                        };
                        out.messages.push(message(id, Role::Assistant, vec![block]));
                    }
                }
                "error" => {
                    let error = item["message"]
                        .as_str()
                        .context("Codex error item missing message")?;
                    out.events
                        .push(SessionEvent::Status(format!("Codex: {error}")));
                    if done {
                        out.messages.push(message(
                            id,
                            Role::Assistant,
                            vec![ContentBlock::text(format!("Codex: {error}"))],
                        ));
                    }
                }
                _ => {
                    let super::tools::ToolProjection {
                        name,
                        input,
                        output,
                        is_error,
                        images,
                    } = super::tools::tool(item)?;
                    if !entry.started {
                        out.events.push(SessionEvent::ToolStart {
                            id: id.clone(),
                            name: name.clone(),
                            input: input.clone(),
                        });
                        out.messages.push(message(
                            format!("{id}:call"),
                            Role::Assistant,
                            vec![ContentBlock::ToolUse {
                                id: id.clone(),
                                name: name.clone(),
                                input,
                            }],
                        ));
                    }
                    if done {
                        out.events.push(SessionEvent::ToolEnd {
                            id: id.clone(),
                            name,
                            output: output.clone(),
                            is_error,
                            images: images.clone(),
                        });
                        out.messages.push(message(
                            format!("{id}:result"),
                            Role::Tool,
                            vec![ContentBlock::ToolResult {
                                tool_use_id: id,
                                content: output,
                                is_error,
                                images,
                            }],
                        ));
                    }
                }
            }
            entry.started = true;
            entry.done = done;
        }
        _ => bail!("unsupported Codex event type: {kind}"),
    }
    Ok((state, out))
}

pub fn interrupt(state: &Decoder, reason: &str) -> Projection {
    let mut out = Projection::default();
    for (source_id, item) in &state.items {
        if item.done {
            continue;
        }
        let id = format!("{}:{source_id}", state.prefix);
        if matches!(item.kind.as_str(), "reasoning" | "agent_message") {
            if !item.text.is_empty() {
                let block = if item.kind == "reasoning" {
                    ContentBlock::Reasoning {
                        text: item.text.clone(),
                    }
                } else {
                    ContentBlock::text(&item.text)
                };
                out.messages.push(message(id, Role::Assistant, vec![block]));
            }
        } else if item.kind != "error" {
            out.events.push(SessionEvent::ToolEnd {
                id: id.clone(),
                name: item.kind.clone(),
                output: reason.into(),
                is_error: true,
                images: vec![],
            });
            out.messages.push(message(
                format!("{id}:result"),
                Role::Tool,
                vec![ContentBlock::ToolResult {
                    tool_use_id: id,
                    content: reason.into(),
                    is_error: true,
                    images: vec![],
                }],
            ));
        }
    }
    out
}
