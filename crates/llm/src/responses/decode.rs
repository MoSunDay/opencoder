//! Pure event reduction. No tool is committed before response.completed.
use super::output::{function_call, item_kind, required_str, text_parts};
use crate::LlmEvent;
use anyhow::{anyhow, bail, Result};
use opencoder_core::ProviderState;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

#[derive(Default)]
struct Item {
    id: Option<String>,
    kind: Option<String>,
    call_id: Option<String>,
    name: Option<String>,
    arguments: String,
    started: bool,
    texts: BTreeMap<usize, String>,
    summaries: BTreeMap<usize, String>,
    done: Option<Value>,
}

pub(crate) struct Decoder {
    scope: ProviderState,
    items: BTreeMap<usize, Item>,
    response_id: Option<String>,
    last_sequence: Option<(u64, Value)>,
    pub completed: bool,
}

impl Decoder {
    pub fn new(scope: ProviderState) -> Self {
        Self {
            scope,
            items: BTreeMap::new(),
            response_id: None,
            last_sequence: None,
            completed: false,
        }
    }

    pub fn push(&mut self, value: Value) -> Result<Vec<LlmEvent>> {
        if self.completed {
            return Ok(vec![]);
        }
        if value["object"] == "response" {
            return self.complete(&value);
        }
        if let Some(sequence) = value["sequence_number"].as_u64() {
            if let Some((last, previous)) = &self.last_sequence {
                if sequence == *last && previous == &value {
                    return Ok(vec![]);
                }
                if sequence <= *last {
                    bail!("Responses stream has inconsistent sequence numbers");
                }
            }
            self.last_sequence = Some((sequence, value.clone()));
        }
        if let Some(id) = value["response_id"]
            .as_str()
            .or_else(|| value.pointer("/response/id").and_then(Value::as_str))
        {
            bind(&mut self.response_id, id, "response_id")?;
        }
        let kind = value["type"]
            .as_str()
            .ok_or_else(|| anyhow!("Responses event missing type"))?;
        match kind {
            "response.completed" => return self.complete(&value["response"]),
            "response.failed" | "response.incomplete" | "response.cancelled" | "error" => {
                bail!("Responses {kind}: {}", error_detail(&value));
            }
            "response.created" | "response.in_progress" | "response.queued" => return Ok(vec![]),
            _ => {}
        }
        let index = value["output_index"]
            .as_u64()
            .ok_or_else(|| anyhow!("Responses event `{kind}` missing output_index"))?
            as usize;
        let item = self.items.entry(index).or_default();
        if let Some(id) = value["item_id"].as_str() {
            bind(&mut item.id, id, "item_id")?;
        }
        let mut events = Vec::new();
        match kind {
            "response.output_item.added" | "response.output_item.done" => {
                let output = &value["item"];
                update_item(item, output, index, &mut events)?;
                if kind.ends_with(".done") {
                    if let Some(done) = &item.done {
                        if done != output {
                            bail!("conflicting completed output item");
                        }
                    }
                    reconcile(item, output, index, &mut events)?;
                    item.done = Some(output.clone());
                }
            }
            "response.function_call_arguments.delta" => {
                let delta = value["delta"]
                    .as_str()
                    .ok_or_else(|| anyhow!("missing arguments delta"))?;
                item.arguments.push_str(delta);
                if item.started {
                    events.push(LlmEvent::ToolCallDelta {
                        index,
                        arguments: delta.into(),
                    });
                }
            }
            "response.function_call_arguments.done" => {
                let full = value["arguments"]
                    .as_str()
                    .ok_or_else(|| anyhow!("missing completed arguments"))?;
                let delta = suffix(&item.arguments, full)?;
                if item.started && !delta.is_empty() {
                    events.push(LlmEvent::ToolCallDelta {
                        index,
                        arguments: delta.into(),
                    });
                }
                item.arguments = full.into();
            }
            "response.output_text.delta"
            | "response.refusal.delta"
            | "response.reasoning_summary_text.delta" => {
                let reasoning = kind.contains("reasoning_summary");
                let part = part_index(&value, reasoning)?;
                let delta = value["delta"]
                    .as_str()
                    .ok_or_else(|| anyhow!("missing text delta"))?;
                let parts = if reasoning {
                    &mut item.summaries
                } else {
                    &mut item.texts
                };
                parts.entry(part).or_default().push_str(delta);
                events.push(text_event(reasoning, delta.into()));
            }
            "response.output_text.done"
            | "response.refusal.done"
            | "response.reasoning_summary_text.done" => {
                let reasoning = kind.contains("reasoning_summary");
                let part = part_index(&value, reasoning)?;
                let field = if kind == "response.refusal.done" {
                    "refusal"
                } else {
                    "text"
                };
                let full = value[field]
                    .as_str()
                    .ok_or_else(|| anyhow!("missing completed text"))?;
                complete_text(
                    if reasoning {
                        &mut item.summaries
                    } else {
                        &mut item.texts
                    },
                    part,
                    full,
                    reasoning,
                    &mut events,
                )?;
            }
            "response.content_part.added"
            | "response.content_part.done"
            | "response.reasoning_summary_part.added"
            | "response.reasoning_summary_part.done"
            | "response.output_text.annotation.added" => {}
            _ => bail!("unsupported Responses event `{kind}`"),
        }
        Ok(events)
    }

    fn complete(&mut self, response: &Value) -> Result<Vec<LlmEvent>> {
        if response["status"] != "completed" {
            bail!(
                "Responses unsuccessful terminal status {}: {}",
                response["status"],
                error_detail(response)
            );
        }
        let output = response["output"]
            .as_array()
            .ok_or_else(|| anyhow!("completed Response missing output"))?;
        if self.items.keys().any(|index| *index >= output.len()) {
            bail!("completed Response omitted streamed output items");
        }
        let mut events = Vec::new();
        let mut calls = Vec::new();
        let mut ids = HashSet::new();
        let mut text = Vec::new();
        let mut has_final = false;
        for (index, output) in output.iter().enumerate() {
            let item = self.items.entry(index).or_default();
            update_item(item, output, index, &mut events)?;
            if let Some(done) = &item.done {
                if done != output {
                    bail!("completed Response disagrees with output_item.done");
                }
            }
            if output
                .get("status")
                .is_some_and(|status| status != "completed")
            {
                bail!("completed Response contains unfinished item");
            }
            reconcile(item, output, index, &mut events)?;
            match item_kind(output)? {
                "function_call" => {
                    let call = function_call(output)?;
                    if !ids.insert(call.id.clone()) {
                        bail!("duplicate function call_id {}", call.id);
                    }
                    calls.push(call);
                }
                "message" => {
                    if output["phase"] != "commentary" {
                        has_final = true;
                    }
                    text.extend(text_parts(output)?.into_iter().map(|(_, text)| text));
                }
                _ => {}
            }
        }
        if calls.is_empty() && !has_final {
            bail!("Response completed without a final answer or tool call");
        }
        let usage = response
            .get("usage")
            .filter(|u| !u.is_null())
            .map(crate::client::parse_response_usage);
        let mut state = self.scope.clone();
        state.output = output.clone();
        events.push(LlmEvent::ProviderState(state));
        events.push(LlmEvent::Completed {
            text: text.join("\n"),
            tool_calls: calls,
            usage,
        });
        self.completed = true;
        Ok(events)
    }
}

fn bind(slot: &mut Option<String>, value: &str, field: &str) -> Result<()> {
    if slot.as_deref().is_some_and(|previous| previous != value) {
        bail!("conflicting Responses {field}");
    }
    *slot = Some(value.into());
    Ok(())
}

fn update_item(
    item: &mut Item,
    value: &Value,
    index: usize,
    events: &mut Vec<LlmEvent>,
) -> Result<()> {
    bind(&mut item.kind, item_kind(value)?, "item type")?;
    if let Some(id) = value["id"].as_str() {
        bind(&mut item.id, id, "item_id")?;
    }
    if value["type"] == "function_call" {
        bind(
            &mut item.call_id,
            required_str(value, "call_id")?,
            "call_id",
        )?;
        bind(
            &mut item.name,
            required_str(value, "name")?,
            "function name",
        )?;
        if !item.started {
            item.started = true;
            events.push(LlmEvent::ToolCallStart {
                index,
                id: item.call_id.clone().unwrap(),
                name: item.name.clone().unwrap(),
            });
            if !item.arguments.is_empty() {
                events.push(LlmEvent::ToolCallDelta {
                    index,
                    arguments: item.arguments.clone(),
                });
            }
        }
    }
    Ok(())
}

fn reconcile(
    item: &mut Item,
    value: &Value,
    index: usize,
    events: &mut Vec<LlmEvent>,
) -> Result<()> {
    match item_kind(value)? {
        "function_call" => {
            let arguments = required_str(value, "arguments")?;
            let delta = suffix(&item.arguments, arguments)?;
            if !delta.is_empty() {
                events.push(LlmEvent::ToolCallDelta {
                    index,
                    arguments: delta.into(),
                });
            }
            item.arguments = arguments.into();
        }
        kind => {
            let reasoning = kind == "reasoning";
            let parts = text_parts(value)?;
            let seen = if reasoning {
                &mut item.summaries
            } else {
                &mut item.texts
            };
            if seen.keys().any(|i| *i >= parts.len()) {
                bail!("completed item omitted streamed content");
            }
            for (index, text) in parts {
                complete_text(seen, index, &text, reasoning, events)?;
            }
        }
    }
    Ok(())
}

fn complete_text(
    parts: &mut BTreeMap<usize, String>,
    index: usize,
    full: &str,
    reasoning: bool,
    events: &mut Vec<LlmEvent>,
) -> Result<()> {
    let partial = parts.entry(index).or_default();
    let delta = suffix(partial, full)?;
    if !delta.is_empty() {
        events.push(text_event(reasoning, delta.into()));
    }
    *partial = full.into();
    Ok(())
}

fn suffix<'a>(partial: &str, full: &'a str) -> Result<&'a str> {
    full.strip_prefix(partial)
        .ok_or_else(|| anyhow!("completed Responses content disagrees with streamed content"))
}
fn part_index(value: &Value, reasoning: bool) -> Result<usize> {
    value[if reasoning {
        "summary_index"
    } else {
        "content_index"
    }]
    .as_u64()
    .map(|v| v as usize)
    .ok_or_else(|| anyhow!("Responses event missing content/summary index"))
}
fn text_event(reasoning: bool, text: String) -> LlmEvent {
    if reasoning {
        LlmEvent::ReasoningDelta(text)
    } else {
        LlmEvent::TextDelta(text)
    }
}
fn error_detail(value: &Value) -> String {
    let detail = value
        .pointer("/response/error")
        .filter(|v| !v.is_null())
        .or_else(|| value.pointer("/response/incomplete_details"))
        .or_else(|| value.get("error").filter(|v| !v.is_null()))
        .or_else(|| value.get("incomplete_details"))
        .or_else(|| value.get("message"));
    detail
        .map(Value::to_string)
        .unwrap_or_else(|| "no error details".into())
}
