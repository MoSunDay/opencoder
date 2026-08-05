use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use opencoder_core::{message::now_ms, ContentBlock, Message, Role, ToolArc};
use opencoder_llm::{estimate_messages, lower_messages, ChatRequest, LlmEvent};
use opencoder_store::SessionPatch;

use crate::prompt::{build_system, compaction_system_prompt, compaction_user_prompt};
use crate::runner::await_cancel;
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
/// The global `~/.opencoder/AGENTS.md` ships in the system prompt (see
/// `build_system`), so its tokens count toward the compaction budget exactly
/// like any other context the model actually consumes.
fn estimated_tokens(session: &SessionState) -> u64 {
    let system = build_system(
        &session.agent,
        &session.working_dir,
        session.skill_prompt_cloned().as_deref(),
        &session.config.capabilities,
    );
    estimate_messages(&session.messages).saturating_add(estimate(&system.text())) as u64
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
        store
            .update_session(
                &session.id,
                &SessionPatch {
                    summary: Some(summary.clone()),
                    summary_seq: Some(new_skip),
                    // Persist the head images that survived compaction so resume
                    // can rebuild the summary message WITHOUT reloading the
                    // soft-deleted compacted head (the fix for long-session
                    // resume stalls).
                    summary_images: Some(preserved.clone()),
                    updated_at: Some(now_ms()),
                    clear_handoff: true,
                    ..Default::default()
                },
            )
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
                images: Vec::new(),
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

    /// Issue #3 (root cause A): the compaction-summary LLM stream must honor
    /// the session cancel token. A double-Esc / web interrupt mid-compaction
    /// must abort promptly and leave the transcript untouched (compaction only
    /// rewrites `messages` after the summary returns Ok).
    #[tokio::test]
    async fn compact_honors_cancel_and_leaves_messages_intact() {
        use std::sync::Arc;

        use opencoder_core::{resolve_agent, Config};
        use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
        use tokio_util::sync::CancellationToken;

        let cancel = CancellationToken::new();
        cancel.cancel();
        let mock: Arc<dyn ChatStream> = Arc::new(MockChatClient::new().with_default(vec![
            LlmEvent::TextDelta("partial ".into()),
            LlmEvent::TextDelta("summary".into()),
            LlmEvent::Completed {
                text: "partial summary".into(),
                tool_calls: Vec::<CompletedToolCall>::new(),
                usage: Some(Usage {
                    input_tokens: 5,
                    output_tokens: 3,
                    total_tokens: 8,
                    ..Default::default()
                }),
            },
        ]));
        let agent = resolve_agent("act").expect("act agent");
        let mut s = SessionState::new(
            "compact-cancel",
            agent,
            Config {
                model: "main/glm-5.2".into(),
                ..Config::default()
            },
            mock,
            std::env::temp_dir(),
        )
        .with_cancel(cancel);
        // Two turns so `compaction_split` returns a real head/tail split.
        s.messages.push(Message::user("u1", "first turn"));
        s.messages.push(Message::assistant("a1"));
        s.messages.push(Message::user("u2", "second turn"));
        s.messages.push(Message::assistant("a2"));
        let before = s.messages.len();

        let mut events: Vec<SessionEvent> = Vec::new();
        let outcome = compact(&mut s, &HashMap::new(), &mut |ev| events.push(ev)).await;

        assert!(outcome.is_err(), "compaction must abort when cancelled");
        assert_eq!(
            s.messages.len(),
            before,
            "transcript must be untouched when compaction is cancelled"
        );
        // No synthetic compaction-summary message was prepended.
        assert!(s
            .messages
            .iter()
            .all(|m| { !(m.synthetic && m.text().starts_with("[Conversation summary so far]")) }));
        // The cancel arm emits an interrupted status before bailing.
        assert!(events
            .iter()
            .any(|ev| matches!(ev, SessionEvent::Status(msg) if msg == "interrupted")));
    }

    #[test]
    fn collect_head_images_gathers_user_and_tool_images() {
        let mut u = Message::user("u1", "hi");
        u.blocks.push(ContentBlock::Image {
            url: "u1.png".into(),
            detail: None,
        });
        let t = Message {
            id: "t1".into(),
            role: Role::Tool,
            blocks: vec![ContentBlock::ToolResult {
                tool_use_id: "tc".into(),
                content: "x".into(),
                is_error: false,
                images: vec!["t1a.png".into(), "t1b.png".into()],
            }],
            model: None,
            agent: None,
            usage: MessageUsage::default(),
            created_at: 0,
            synthetic: false,
        };
        let imgs = collect_head_images(&[u, t]);
        assert_eq!(imgs, vec!["u1.png", "t1a.png", "t1b.png"]);
    }

    #[test]
    fn collect_head_images_caps_at_max_keeping_most_recent() {
        let mut msgs = Vec::new();
        for i in 0..(MAX_PRESERVED_IMAGES + 2) {
            let mut m = Message::user(format!("u{i}"), "x");
            m.blocks.push(ContentBlock::Image {
                url: format!("img{i}.png"),
                detail: None,
            });
            msgs.push(m);
        }
        let imgs = collect_head_images(&msgs);
        assert_eq!(imgs.len(), MAX_PRESERVED_IMAGES);
        // newest = the last MAX_PRESERVED_IMAGES images
        assert_eq!(imgs[0], "img2.png");
        assert_eq!(
            imgs.last().unwrap(),
            &format!("img{}.png", MAX_PRESERVED_IMAGES + 1)
        );
    }

    #[test]
    fn collect_head_images_empty_is_empty() {
        assert!(collect_head_images(&[]).is_empty());
        assert!(collect_head_images(&[Message::user("u1", "no image")]).is_empty());
    }

    #[test]
    fn strip_images_removes_image_blocks_and_keeps_text() {
        let mut m = Message::user("u1", "hello");
        m.blocks.push(ContentBlock::Image {
            url: "x.png".into(),
            detail: None,
        });
        let stripped = strip_images(&[m]);
        assert_eq!(stripped.len(), 1);
        assert!(
            !stripped[0]
                .blocks
                .iter()
                .any(|b| matches!(b, ContentBlock::Image { .. })),
            "Image blocks must be stripped"
        );
        assert!(!stripped[0].has_image());
        assert!(stripped[0].text().contains("hello"));
    }

    #[test]
    fn strip_images_clears_tool_result_images() {
        let m = Message {
            id: "t1".into(),
            role: Role::Tool,
            blocks: vec![ContentBlock::ToolResult {
                tool_use_id: "tc".into(),
                content: "shot".into(),
                is_error: false,
                images: vec!["shot.png".into()],
            }],
            model: None,
            agent: None,
            usage: MessageUsage::default(),
            created_at: 0,
            synthetic: false,
        };
        let stripped = strip_images(&[m]);
        match &stripped[0].blocks[0] {
            ContentBlock::ToolResult {
                images, content, ..
            } => {
                assert!(images.is_empty(), "tool images must be cleared");
                assert_eq!(content, "shot");
            }
            other => panic!("unexpected block: {other:?}"),
        }
    }

    /// RC4: images in the compacted head must survive compaction by attaching
    /// to the summary message, so the (vision-capable) main model still sees
    /// them after summarization. Deterministic, zero-network.
    #[tokio::test]
    async fn compaction_preserves_head_images_on_summary_message() {
        use std::sync::Arc;

        use opencoder_core::resolve_agent;
        use opencoder_core::Config;
        use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};

        let mock: Arc<dyn ChatStream> = Arc::new(MockChatClient::new().with_default(vec![
            LlmEvent::TextDelta("summary of talk".into()),
            LlmEvent::Completed {
                text: "summary of talk".into(),
                tool_calls: Vec::<CompletedToolCall>::new(),
                usage: Some(Usage {
                    input_tokens: 1,
                    output_tokens: 1,
                    total_tokens: 2,
                    ..Default::default()
                }),
            },
        ]));
        let agent = resolve_agent("act").expect("act agent");
        let mut s = SessionState::new(
            "compact-img",
            agent,
            Config {
                model: "main/glm-5.2".into(),
                ..Config::default()
            },
            mock,
            std::env::temp_dir(),
        );
        // Two turns; the head (u1+a1) carries an image, the tail (u2+a2) does not.
        let mut u1 = Message::user("u1", "look at this");
        u1.blocks.push(ContentBlock::Image {
            url: "data:image/png;base64,AAAA".into(),
            detail: None,
        });
        s.messages.push(u1);
        s.messages.push(Message::assistant("a1"));
        s.messages.push(Message::user("u2", "second"));
        s.messages.push(Message::assistant("a2"));

        let mut events: Vec<SessionEvent> = Vec::new();
        let outcome = compact(&mut s, &HashMap::new(), &mut |ev| events.push(ev)).await;
        assert!(outcome.is_ok(), "compaction must succeed: {outcome:?}");

        // The summary message (now messages[0]) must carry the preserved image.
        assert!(!s.messages.is_empty());
        let summary = &s.messages[0];
        assert_eq!(summary.role, Role::User);
        assert!(
            summary.text().starts_with("[Conversation summary so far]"),
            "summary text prefix intact"
        );
        assert!(
            summary.has_image(),
            "summary message must preserve the head image"
        );
        // And lowering yields a legal multimodal user turn with image_url.
        let lowered = opencoder_llm::lower_messages(&s.messages);
        let user_img = lowered
            .iter()
            .find(|m| m["role"] == "user" && m["content"].is_array())
            .expect("a lowered user message carrying an image");
        let content = user_img["content"].as_array().unwrap();
        assert!(content
            .iter()
            .any(|p| p["type"] == "image_url"
                && p["image_url"]["url"] == "data:image/png;base64,AAAA"));
    }
}
