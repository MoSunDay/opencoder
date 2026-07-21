use std::collections::HashMap;

use anyhow::{anyhow, Result};
use opencoder_core::{message::now_ms, Message, Role, ToolArc};
use opencoder_llm::{estimate_messages, lower_messages, ChatRequest, LlmEvent};
use opencoder_store::SessionPatch;

use crate::prompt::{build_system, compaction_system_prompt, compaction_user_prompt};
use crate::runner::SessionEvent;
use crate::SessionState;

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
///
/// The ambient global `~/.opencoder/AGENTS.md` is excluded: it still ships in
/// the system prompt but is treated as free baseline context that does not
/// count against the session's compaction budget.
fn estimated_tokens(session: &SessionState) -> u64 {
    let system = build_system(
        &session.agent,
        &session.working_dir,
        session.skill_prompt_cloned().as_deref(),
        &session.config.capabilities,
    );
    let mut est = estimate_messages(&session.messages).saturating_add(estimate(&system.text()));
    if let Some(global) = crate::prompt::global_instructions_text(&session.working_dir) {
        est = est.saturating_sub(estimate(&global));
    }
    est as u64
}

fn estimate(s: &str) -> usize {
    opencoder_llm::estimate(s)
}

/// Provider-reported input tokens from the last call. Uses `input_tokens`
/// (not `total_tokens`) so output-heavy turns don't prematurely trip the
/// input budget.
///
/// The global `~/.opencoder/AGENTS.md` footprint is subtracted so this
/// authoritative signal stays consistent with the estimate path (which also
/// excludes the global file) — otherwise a large global file would re-enter
/// the budget here and trip compaction early. The default config leaves a
/// `context_limit − budget` margin (128k − 80k = 48k tokens) that absorbs any
/// realistic global instructions file against overflow.
fn reported_tokens(session: &SessionState) -> u64 {
    let raw = session.last_usage.input_tokens;
    match crate::prompt::global_instructions_text(&session.working_dir) {
        Some(global) => raw.saturating_sub(estimate(&global) as u64),
        None => raw,
    }
}

pub async fn compact(
    session: &mut SessionState,
    _registry: &HashMap<String, ToolArc>,
    on_event: &mut (impl FnMut(SessionEvent) + Send + ?Sized),
) -> Result<Option<String>> {
    let tail = session.config.compaction.tail_turns.max(1) as usize;
    let Some(split) = compaction_split(&session.messages, tail) else {
        // Genuinely nothing to summarize (empty or single-message transcript).
        on_event(SessionEvent::Status("nothing to compact yet".into()));
        return Ok(None);
    };
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

    // Persist compaction summary to the store so resume can reconstruct
    // the compacted transcript instead of reloading the full history.
    if let Some(store) = &session.store {
        let prev_skip = session.summary_seq.unwrap_or(0);
        // The head in the in-memory list is messages[0..split].
        // If there was a previous compaction, the first message is the old
        // summary (not in the store), so the number of STORE messages in
        // the head is split-1. Otherwise all split head messages are in
        // the store.
        let head_store_msgs = if prev_skip > 0 { split - 1 } else { split };
        let new_skip = prev_skip + head_store_msgs as i64;
        let _ = store
            .update_session(
                &session.id,
                &SessionPatch {
                    summary: Some(summary.clone()),
                    summary_seq: Some(new_skip),
                    updated_at: Some(now_ms()),
                    ..Default::default()
                },
            )
            .await;
        session.after_compaction(summary.clone(), new_skip);
    }

    on_event(SessionEvent::Status(String::new()));
    Ok(Some(summary))
}

/// Indices that delimit summarizable conversation turns (used by both the
/// ideal-turn split and the over-budget fallback).
///
/// A message is a turn start when it is:
///   - the first message (index 0), or
///   - a real (non-synthetic) user message, or
///   - an assistant message that follows a tool message (the model's fresh
///     response after consuming tool results — a new cycle within a single
///     user request, common in tool-intensive coding sessions).
///
/// This generalization ensures compaction fires for single-user tasks that
/// accumulate many tool roundtrips — the most common coding-agent shape —
/// without changing the split point for classic multi-user sessions (where
/// every turn start is already a real user message, so the set is identical).
fn turn_start_indices(messages: &[Message]) -> Vec<usize> {
    messages
        .iter()
        .enumerate()
        .filter(|(i, m)| {
            *i == 0
                || (m.role == Role::User && !m.synthetic)
                || (m.role == Role::Assistant && *i > 0 && messages[i - 1].role == Role::Tool)
        })
        .map(|(i, _)| i)
        .collect()
}

/// Ideal split point: keep `tail_turns` recent turns as the tail. Returns 0
/// when there are too few turns to split while preserving any tail — the
/// caller (`compact`) applies a progress-guaranteeing fallback via
/// `compaction_split` in that case.
#[cfg_attr(not(test), allow(dead_code))]
fn split_index(messages: &[Message], tail_turns: usize) -> usize {
    let turn_starts = turn_start_indices(messages);
    if turn_starts.len() <= tail_turns {
        return 0;
    }
    turn_starts[turn_starts.len() - tail_turns]
}

/// Resolve the head/tail split for compaction. Unlike `split_index` (the
/// *ideal* turn-aware split, which returns 0 when there are too few turns),
/// this guarantees forward progress when the transcript is over budget:
/// instead of bailing out it falls back to summarizing the oldest turn (or,
/// for a single conversation turn, everything except the most recent message),
/// so an oversized short-turn conversation is still compressed rather than
/// shipped to the model verbatim.
///
/// Returns `None` only when there is genuinely nothing to summarize — an
/// empty transcript or a single message.
fn compaction_split(messages: &[Message], tail_turns: usize) -> Option<usize> {
    let turn_starts = turn_start_indices(messages);
    if turn_starts.is_empty() {
        return None;
    }
    // Ideal: keep `tail_turns` recent turns as the tail.
    if turn_starts.len() > tail_turns {
        return Some(turn_starts[turn_starts.len() - tail_turns]);
    }
    // Fewer turns than tail_turns, but we are over budget (the caller only
    // invokes compaction when `should_compact` fired). Summarize the oldest
    // turn and keep every subsequent turn as the tail.
    if turn_starts.len() >= 2 {
        return Some(turn_starts[1]);
    }
    // A single conversation turn. Keep the most recent message intact and
    // summarize whatever precedes it (if anything).
    (messages.len() > 1).then_some(1)
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
        cache_salt: crate::cache_salt_for(session),
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

pub(crate) fn compaction_message(summary: String) -> Message {
    let mut m = Message::user(
        crate::runner::new_id(),
        format!("[Conversation summary so far]\n{summary}"),
    );
    m.synthetic = true;
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_core::{ContentBlock, MessageUsage};

    fn tool_msg(id: &str, tool_use_id: &str) -> Message {
        Message {
            id: id.into(),
            role: Role::Tool,
            blocks: vec![ContentBlock::ToolResult {
                tool_use_id: tool_use_id.into(),
                content: "x".into(),
                is_error: false,
            }],
            model: None,
            agent: None,
            usage: MessageUsage::default(),
            created_at: 0,
            synthetic: false,
        }
    }

    fn assistant_with_tool(id: &str) -> Message {
        let mut m = Message::assistant(id);
        m.blocks.push(ContentBlock::ToolUse {
            id: "tc".into(),
            name: "bash".into(),
            input: serde_json::json!({}),
        });
        m
    }

    #[test]
    fn split_index_assistant_after_tool_is_turn_boundary() {
        // Single user task with 3 tool roundtrips — common coding-agent shape.
        // With the old code this would return 0 (only 1 real user message).
        let msgs = vec![
            Message::user("u1", "task"),
            assistant_with_tool("a1"),
            tool_msg("t1", "tc"),
            assistant_with_tool("a2"),
            tool_msg("t2", "tc"),
            assistant_with_tool("a3"),
            tool_msg("t3", "tc"),
            Message::assistant("a4"),
        ];
        // turn_starts = [0, 3, 5, 7], tail=2 → split = turn_starts[2] = 5
        let split = split_index(&msgs, 2);
        assert!(
            split > 0,
            "tool-intensive single-user session must be splittable, got split={split}"
        );
        assert_eq!(split, 5);
    }

    #[test]
    fn split_index_multi_user_unchanged() {
        // Classic multi-user session — split point must not change.
        let msgs = vec![
            Message::user("u1", "first task"),
            Message::assistant("a1"),
            Message::user("u2", "second task"),
            Message::assistant("a2"),
            Message::user("u3", "third task"),
            Message::assistant("a3"),
        ];
        // turn_starts = [0, 2, 4] (all real user messages)
        // tail=2 → split = turn_starts[1] = 2
        assert_eq!(split_index(&msgs, 2), 2);
        // tail=1 → split = turn_starts[2] = 4
        assert_eq!(split_index(&msgs, 1), 4);
    }

    #[test]
    fn split_index_returns_zero_when_too_few_turns() {
        // Single user + one tool roundtrip → turn_starts=[0, 3], tail=2 → 0.
        let msgs = vec![
            Message::user("u1", "task"),
            assistant_with_tool("a1"),
            tool_msg("t1", "tc"),
            Message::assistant("a2"),
        ];
        assert_eq!(split_index(&msgs, 2), 0);
    }

    #[test]
    fn split_index_mixed_user_and_tool_turns() {
        // A session with both real user turns and tool roundtrips.
        let msgs = vec![
            Message::user("u1", "task1"),
            assistant_with_tool("a1"),
            tool_msg("t1", "tc"),
            assistant_with_tool("a2"),
            tool_msg("t2", "tc"),
            Message::user("u2", "task2"),
            assistant_with_tool("a3"),
            tool_msg("t3", "tc"),
            Message::assistant("a4"),
        ];
        // turn_starts = [0, 3, 5, 8], tail=2 → split = turn_starts[2] = 5
        assert_eq!(split_index(&msgs, 2), 5);
        // tail=1 → split = turn_starts[3] = 8
        assert_eq!(split_index(&msgs, 1), 8);
    }

    #[test]
    fn compaction_split_fallback_summarizes_oldest_turn() {
        // Two turns, tail_turns=2: ideal split_index returns 0 (too few
        // turns), but the over-budget fallback must still split — summarizing
        // the first turn and keeping the second.
        // turn_starts = [0, 2], fallback -> turn_starts[1] = 2.
        let msgs = vec![
            Message::user("u1", "first"),
            Message::assistant("a1"),
            Message::user("u2", "second"),
            Message::assistant("a2"),
        ];
        assert_eq!(compaction_split(&msgs, 2), Some(2));
        // head = msgs[..2] (first turn), tail = msgs[2..] (second turn).
    }

    #[test]
    fn compaction_split_fallback_two_tool_turns() {
        // turn_starts = [0, 3], tail_turns=2 -> ideal returns 0; fallback
        // -> turn_starts[1] = 3 (keep the second turn, summarize the first).
        let msgs = vec![
            Message::user("u1", "task"),
            assistant_with_tool("a1"),
            tool_msg("t1", "tc"),
            Message::user("u2", "more"),
            Message::assistant("a2"),
        ];
        assert_eq!(compaction_split(&msgs, 2), Some(3));
    }

    #[test]
    fn compaction_split_single_turn_keeps_last_message() {
        // One turn (turn_starts=[0]), two messages: summarize the first
        // message, keep the most recent one as the tail.
        let msgs = vec![Message::user("u1", "big paste"), Message::assistant("a1")];
        assert_eq!(compaction_split(&msgs, 2), Some(1));
    }

    #[test]
    fn compaction_split_single_message_is_no_op() {
        // A lone message cannot be summarized without destroying the only
        // context — this is the one genuine no-op.
        let msgs = vec![Message::user("u1", "big paste")];
        assert_eq!(compaction_split(&msgs, 2), None);
        assert_eq!(compaction_split(&[], 2), None);
    }

    #[test]
    fn compaction_split_matches_ideal_when_enough_turns() {
        // Three turns, tail_turns=2 -> ideal path equals split_index.
        let msgs = vec![
            Message::user("u1", "a"),
            Message::assistant("a1"),
            Message::user("u2", "b"),
            Message::assistant("a2"),
            Message::user("u3", "c"),
            Message::assistant("a3"),
        ];
        // turn_starts = [0, 2, 4]; tail=2 -> turn_starts[1] = 2
        assert_eq!(compaction_split(&msgs, 2), Some(2));
        assert_eq!(compaction_split(&msgs, 2).unwrap(), split_index(&msgs, 2));
    }
}
