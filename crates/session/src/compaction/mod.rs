use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use opencoder_core::{message::now_ms, AgentKind, ContentBlock, Message, Role, ToolArc};
use opencoder_llm::{estimate_messages, lower_messages, ChatRequest, LlmEvent};
use opencoder_store::SessionPatch;

use crate::prompt::{build_system, compaction_system_prompt, compaction_user_prompt};
use crate::runner::{await_cancel, SessionEvent};
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
    let reported = reported_tokens(session);
    tracing::debug!(
        estimated,
        reported,
        budget,
        "should_compact: estimated vs reported vs budget"
    );
    if estimated >= budget {
        return true;
    }
    reported != 0 && reported >= budget
}

/// Estimated tokens of the conversation about to be sent (system + messages).
///
/// The global `~/.opencoder/AGENTS.md` ships in the system prompt (see
/// `build_system`), so its tokens count toward the compaction budget exactly
/// like any other context the model actually consumes.
fn estimated_tokens(session: &SessionState) -> u64 {
    let skill = session.skill_prompt_cloned();
    let mcp_status: Vec<_> = crate::mcp::pool::status_for(&session.id)
        .into_iter()
        .filter(|(name, _)| {
            session
                .config
                .enabled_mcp_servers_for(&session.agent.name, session.agent.mode)
                .iter()
                .any(|(enabled, _)| enabled == name)
        })
        .collect();
    let mcp = crate::prompt::mcp_section(&mcp_status);
    let agent = &session.agent;
    let cli = crate::prompt::cli_section(&session.config.enabled_cli_for(&agent.name, agent.mode));
    let runtime = crate::prompt::runtime_sections(mcp.as_deref(), cli.as_deref());
    let system = build_system(&session.agent, &session.working_dir, runtime.as_deref());
    let base = estimate_messages(&session.messages)
        .saturating_add(estimate(&system.text()))
        .saturating_add(
            crate::skill_context::tail_reminder(session)
                .map(|m| estimate(&m.text()))
                .unwrap_or(0),
        );
    let registry = crate::tools::registry();
    let tool_tokens =
        crate::tools::estimate_tool_schema_tokens(&session.agent, skill.as_deref(), &registry);
    base.saturating_add(tool_tokens) as u64
}

fn estimate(s: &str) -> usize {
    opencoder_llm::estimate(s)
}

/// Provider-reported input tokens from the last call. Uses `input_tokens`
/// (not `total_tokens`) so output-heavy turns don't prematurely trip the
/// input budget. The value already includes the global instructions file
/// (it ships in the system prompt), so no adjustment is needed — it stays
/// consistent with the estimate path, which also counts it.
fn reported_tokens(session: &SessionState) -> u64 {
    session.last_usage.input_tokens
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

    // Strip image content from the summarization input: the (possibly
    // non-vision) small_model cannot consume `image_url` parts, and the head
    // images are preserved separately on the summary message below rather than
    // described in prose. Text/tool content is unchanged.
    let head_for_summary = strip_images(&head);
    let summary = summarize(
        &head_for_summary,
        session,
        previous_summary.as_deref(),
        on_event,
    )
    .await?;
    let mut summary_msg = compaction_message(summary.clone());
    // Preserve recent images from the discarded head by attaching them to the
    // summary message. They then travel with the summary as legal `image_url`
    // parts in the (vision-capable) main model's context. Keeping them on the
    // single summary message (instead of a separate synthetic turn) leaves the
    // `summary_seq` accounting unchanged: the summary is still exactly one
    // in-memory-only message.
    let preserved = collect_head_images(&head);
    if !preserved.is_empty() {
        for url in &preserved {
            summary_msg.blocks.push(ContentBlock::Image {
                url: url.clone(),
                detail: None,
            });
        }
    }
    // Plan snapshot capture: in plan mode the final plan is an assistant
    // message that may live in the head being folded into the user-role
    // summary. Snapshot it BEFORE `session.messages` is replaced so a later
    // plan→act handoff still finds the plan (`final_plan_text` only scans
    // the live tail). On miss, keep any existing snapshot — an earlier
    // compaction may already hold the newest plan text.
    if session.agent.kind == AgentKind::Plan {
        if let Some(plan) = crate::plan_handoff::final_plan_text(&session.messages) {
            session.plan_snapshot = Some(plan);
        }
    }
    let tail_msgs: Vec<Message> = session.messages[split..].to_vec();
    session.messages = vec![summary_msg].into_iter().chain(tail_msgs).collect();

    // Persist compaction summary to the store so resume can reconstruct
    // the compacted transcript instead of reloading the full history.
    //
    // Compute the new skip BEFORE calling after_compaction (which clears
    // handoff_seq). Bookkeeping is updated BEFORE the DB write so the in-memory
    // state stays coherent even if persistence fails — the error is propagated
    // so the caller (run_loop) can retry or surface it.
    let prev_skip = session.summary_seq.or(session.handoff_seq).unwrap_or(0);
    let head_store_msgs = if prev_skip > 0 || session.handoff_seq.is_some() {
        split.saturating_sub(1)
    } else {
        split
    };
    let new_skip = prev_skip + head_store_msgs as i64;
    session.after_compaction(summary.clone(), new_skip);
    session.summary_images = preserved.clone();
    if let Some(store) = &session.store {
        let mut patch = SessionPatch {
            summary: Some(summary.clone()),
            summary_seq: Some(new_skip),
            // Persist the head images that survived compaction so resume
            // can rebuild the summary message WITHOUT reloading the
            // soft-deleted compacted head (the fix for long-session
            // resume stalls).
            summary_images: Some(preserved.clone()),
            updated_at: Some(now_ms()),
            clear_handoff: true,
            // Mirror the plan phase so a resumed plan session keeps its
            // arming (counter) and compaction-captured plan snapshot.
            plan_input_count: Some(session.plan_input_count as i64),
            ..Default::default()
        };
        match &session.plan_snapshot {
            Some(snap) => patch.plan_snapshot = Some(snap.clone()),
            None => patch.clear_plan_snapshot = true,
        }
        store
            .update_session(&session.id, &patch)
            .await
            .context("persist compaction metadata")?;
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
    // Cancel guard only: the event-level idle watchdog now lives inside the
    // streaming client, which retries stalls transparently. A double-Esc / web
    // interrupt during the compaction-summary stream must still break out
    // promptly instead of blocking the runner. On cancel we abandon the
    // summary -- `compact` only rewrites `session.messages` AFTER this returns
    // Ok, so abandoning leaves the transcript untouched.
    let mut cancel_fut = std::pin::pin!(await_cancel(session));
    loop {
        tokio::select! {
            biased;
            _ = &mut cancel_fut => {
                on_event(SessionEvent::Status("interrupted".into()));
                return Err(anyhow!("cancelled"));
            }
            ev = rx.recv() => {
                let ev = match ev { Some(ev) => ev, None => break };
                match ev {
                    LlmEvent::TextDelta(t) => {
                        text.push_str(&t);
                        on_event(SessionEvent::CompactionDelta(t));
                    }
                    LlmEvent::Completed { text: t, .. } => {
                        if !t.is_empty() {
                            text = t;
                        }
                    }
                    LlmEvent::Retrying { .. } => {
                        // Mid-stream retry: the client discarded its partial
                        // summary and regenerates from scratch. Drop deltas
                        // accumulated so far so the two attempts aren't
                        // concatenated; the final `Completed` overwrites `text`.
                        text.clear();
                    }
                    LlmEvent::Error(e) => return Err(anyhow!(e)),
                    _ => {}
                }
            }
        }
    }
    if text.trim().is_empty() {
        return Err(anyhow!("empty compaction summary"));
    }
    Ok(text)
}

/// Maximum number of images preserved from a compacted or handed-off head.
/// Keeping a bounded recent set avoids unbounded context growth while ensuring
/// key visual context survives summarization. `estimate_chars` already bills
/// ~256 tokens per image, so this caps the added cost near ~1k tokens.
const MAX_PRESERVED_IMAGES: usize = 4;

/// Collect image URIs from messages that are about to be discarded by
/// compaction or plan->act handoff -- both user-attached `Image` blocks and
/// tool-returned `ToolResult.images`. Keeps the most recent
/// `MAX_PRESERVED_IMAGES` (newest last) so the freshest visual context
/// survives while older ones are summarized in prose.
pub(crate) fn collect_head_images(head: &[Message]) -> Vec<String> {
    let mut all: Vec<String> = Vec::new();
    for m in head {
        for b in &m.blocks {
            match b {
                ContentBlock::Image { url, .. } => all.push(url.clone()),
                ContentBlock::ToolResult { images, .. } => {
                    all.extend(images.iter().cloned());
                }
                _ => {}
            }
        }
    }
    if all.len() > MAX_PRESERVED_IMAGES {
        all.split_off(all.len() - MAX_PRESERVED_IMAGES)
    } else {
        all
    }
}

/// Strip all image content from a message slice -- both `Image` blocks and the
/// `images` vec on `ToolResult` -- returning clones safe to send to a possibly
/// non-vision summarizer model. Text and tool content is left unchanged so the
/// summary still reflects what was said.
fn strip_images(messages: &[Message]) -> Vec<Message> {
    messages
        .iter()
        .map(|m| {
            let mut stripped = m.clone();
            stripped
                .blocks
                .retain(|b| !matches!(b, ContentBlock::Image { .. }));
            for b in &mut stripped.blocks {
                if let ContentBlock::ToolResult { images, .. } = b {
                    images.clear();
                }
            }
            stripped
        })
        .collect()
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
mod image_tests;
#[cfg(test)]
mod tests;
