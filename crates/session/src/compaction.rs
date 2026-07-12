use std::collections::HashMap;

use anyhow::{anyhow, Result};
use opencode_core::{Message, Role, ToolArc};
use opencode_llm::{estimate_messages, lower_messages, ChatRequest, LlmEvent};

use crate::prompt::{build_system, compaction_system_prompt, compaction_user_prompt};
use crate::SessionState;
use crate::runner::SessionEvent;

/// Decide whether to compact. Two signals are checked: the estimated tokens
/// of the transcript (works on round 1, before any usage) and the model-reported
/// usage from the last call (authoritative when present).
///
/// Triggers when either exceeds its budget, where the input budget is
/// `min(context_threshold, context_limit - reserved)` — so `reserved` actually
/// shrinks the usable window (it is no longer dead config).
pub fn should_compact(session: &SessionState) -> bool {
    let cfg = &session.config.compaction;
    if !cfg.auto {
        return false;
    }
    let context_limit = session.config.context_limit();
    let reserved = cfg.reserved.min(context_limit.saturating_sub(1));
    let usable_input = context_limit.saturating_sub(reserved);
    let budget = cfg.context_threshold.min(usable_input);

    let estimated = estimated_tokens(session);
    if estimated >= budget {
        return true;
    }
    let reported = reported_tokens(session);
    reported != 0 && reported >= budget
}

/// Estimated tokens of the conversation about to be sent (system + messages).
fn estimated_tokens(session: &SessionState) -> u64 {
    let system = build_system(
        &session.agent,
        &session.working_dir,
        session.skill_prompt.as_deref(),
    );
    let est = estimate_messages(&session.messages).saturating_add(estimate(&system.text()));
    est as u64
}

fn estimate(s: &str) -> usize {
    opencode_llm::estimate(s)
}

/// Provider-reported input tokens from the last call. Uses `input_tokens`
/// (not `total_tokens`) so output-heavy turns don't prematurely trip the
/// input budget.
fn reported_tokens(session: &SessionState) -> u64 {
    session.last_usage.input_tokens
}

pub async fn compact(
    session: &mut SessionState,
    _registry: &HashMap<String, ToolArc>,
    on_event: &mut (impl FnMut(SessionEvent) + Send + ?Sized),
) -> Result<Option<String>> {
    let tail = session.config.compaction.tail_turns.max(1) as usize;
    let split = split_index(&session.messages, tail);
    if split == 0 {
        on_event(SessionEvent::Status("nothing to compact yet".into()));
        return Ok(None);
    }
    on_event(SessionEvent::Status("compacting conversation…".into()));
    let head: Vec<Message> = session.messages[..split].to_vec();

    // If a previous compaction summary exists in the head, extract its text so
    // the summarizer can incrementally update it rather than starting fresh.
    let previous_summary: Option<String> = head
        .iter()
        .find(|m| {
            m.synthetic
                && m.role == Role::User
                && m.text().starts_with("[Conversation summary so far]\n")
        })
        .map(|m| {
            let text = m.text();
            text.strip_prefix("[Conversation summary so far]\n")
                .unwrap_or(&text)
                .to_string()
        });

    let summary = summarize(&head, session, previous_summary.as_deref(), on_event).await?;
    let summary_msg = compaction_message(summary.clone());
    let tail_msgs: Vec<Message> = session.messages[split..].to_vec();
    session.messages = vec![summary_msg].into_iter().chain(tail_msgs).collect();
    on_event(SessionEvent::Status(String::new()));
    Ok(Some(summary))
}

fn split_index(messages: &[Message], tail_turns: usize) -> usize {
    let user_idx: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == Role::User && !m.synthetic)
        .map(|(i, _)| i)
        .collect();
    if user_idx.len() <= tail_turns {
        return 0;
    }
    user_idx[user_idx.len() - tail_turns]
}

async fn summarize(
    head: &[Message],
    session: &SessionState,
    previous_summary: Option<&str>,
    on_event: &mut (impl FnMut(SessionEvent) + Send + ?Sized),
) -> Result<String> {
    let mut msgs: Vec<serde_json::Value> = Vec::new();
    // System prompt: anchored context summarization assistant.
    msgs.push(serde_json::json!({ "role": "system", "content": compaction_system_prompt() }));
    // The conversation head to summarize.
    msgs.extend(lower_messages(head));
    // User prompt: structured output template (+ optional previous-summary).
    msgs.push(
        serde_json::json!({ "role": "user", "content": compaction_user_prompt(previous_summary) }),
    );
    // Summarization is a cheap background call → use small_model when configured.
    let model = session.config.small_model_or_primary().to_string();
    let req = ChatRequest {
        model,
        messages: msgs,
        tools: Vec::new(),
        tool_choice: None,
        temperature: Some(0.2),
        max_tokens: session.config.compaction.buffer,
        reasoning_effort: None,
    };
    let mut rx = session.client.chat_stream(req)?;
    let mut text = String::new();
    while let Some(ev) = rx.recv().await {
        match ev {
            LlmEvent::TextDelta(t) => {
                text.push_str(&t);
                on_event(SessionEvent::TextDelta(t));
            }
            LlmEvent::Completed { text: t, .. } => {
                if !t.is_empty() {
                    text = t;
                }
            }
            LlmEvent::Error(e) => return Err(anyhow!(e)),
            _ => {}
        }
    }
    if text.trim().is_empty() {
        return Err(anyhow!("empty compaction summary"));
    }
    Ok(text)
}

fn compaction_message(summary: String) -> Message {
    let mut m = Message::user(
        crate::runner::new_id(),
        format!("[Conversation summary so far]\n{summary}"),
    );
    m.synthetic = true;
    m
}
