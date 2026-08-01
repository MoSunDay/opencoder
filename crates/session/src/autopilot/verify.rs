//! Shadow VERIFY sub-agent: an isolated one-shot judgement that NEVER touches
//! the main transcript.
//!
//! This is the same pattern `generate_title_inner` / compaction already use
//! (`lower_messages(snapshot) -> ChatRequest -> chat_stream -> collect`), but
//! explicitly never calls [`SessionState::record`]. The snapshot is built in
//! memory, sent once, parsed, and dropped — so the judgement exchange never
//! pollutes `session.messages` and cannot affect subsequent turns.

use std::sync::Arc;

use anyhow::Result;
use opencoder_core::Message;
use opencoder_llm::{lower_messages, ChatRequest, ChatStream, LlmEvent};

use crate::autopilot::decision::parse_verdict;
use crate::autopilot::prompts::{verify_system_prompt, verify_user_prompt};
use crate::autopilot::state::{ApState, VerifyVerdict};
use crate::runner::new_id;
use crate::SessionState;

/// Headroom reserved out of the context window for the judge system prompt, the
/// goal question and the model's output tokens. The remaining budget is what
/// the cloned transcript may occupy.
const VERIFY_RESERVED_TOKENS: u64 = 2_000;
/// Per-message structural overhead, matching `opencoder_llm::estimate_messages`.
const MSG_OVERHEAD: usize = 4;

/// Run up to `retries` isolated one-shot VERIFY calls against a throwaway
/// snapshot of the current transcript. Returns the first parseable verdict, or
/// [`VerifyVerdict::Malformed`] if none parse.
///
/// `session.messages` is read-only here (immutable borrow) — nothing is
/// recorded or persisted. A transient LLM error counts as a malformed attempt
/// and is retried within the budget.
pub async fn verify(session: &SessionState, state: &ApState, retries: u32) -> VerifyVerdict {
    let msgs = lower_messages(&build_snapshot(session, state));

    for _ in 0..retries {
        let req = ChatRequest {
            model: session.config.small_model_or_primary().to_string(),
            messages: msgs.clone(),
            tools: Vec::new(),
            tool_choice: None,
            temperature: Some(0.0),
            max_tokens: Some(8),
            reasoning_effort: None,
            cache_salt: None,
        };
        match drain_one_shot(&session.client, req).await {
            Ok(text) => match parse_verdict(&text) {
                // Affirmative ("yes") → the goal is fully achieved.
                Some(true) => return VerifyVerdict::Complete,
                // Negative ("no") → more work is still needed.
                Some(false) => return VerifyVerdict::MoreWork,
                None => continue, // malformed → retry within budget
            },
            Err(_) => continue, // transient error → retry within budget
        }
    }
    VerifyVerdict::Malformed
}

/// Build the ephemeral snapshot: judge system prompt + a truncated clone of the
/// transcript + the goal question.
///
/// The transcript is capped to the most recent messages that fit
/// `context_limit - VERIFY_RESERVED_TOKENS` (estimated tokens), so a long
/// autopilot run never overflows the small model's window. The goal is
/// re-stated verbatim in the question, so dropping old turns never loses the
/// anchor the judge is measured against.
fn build_snapshot(session: &SessionState, state: &ApState) -> Vec<Message> {
    let budget = session
        .config
        .context_limit()
        .saturating_sub(VERIFY_RESERVED_TOKENS) as usize;
    let mut snapshot = Vec::with_capacity(session.messages.len() + 2);
    snapshot.push(Message::system(new_id(), verify_system_prompt()));
    if opencoder_llm::estimate_messages(&session.messages) <= budget {
        snapshot.extend(session.messages.iter().cloned());
    } else {
        // Sliding window: walk from the most recent message backward, keeping
        // as many as fit the budget, then restore original order.
        let mut kept: Vec<Message> = Vec::new();
        let mut cost = 0usize;
        for m in session.messages.iter().rev() {
            let msg_cost = opencoder_llm::estimate(&m.estimate_chars()) + MSG_OVERHEAD;
            if cost + msg_cost > budget {
                break;
            }
            cost += msg_cost;
            kept.push(m.clone());
        }
        kept.reverse();
        snapshot.extend(kept);
    }
    snapshot.push(Message::user(new_id(), verify_user_prompt(&state.goal)));
    snapshot
}

/// Collect a single completion into a String. Mirrors the sink loop in
/// `generate_title_inner`: accumulate deltas, snap to the terminal `Completed`
/// text, surface stream errors. Returns the final assistant text (possibly
/// empty).
async fn drain_one_shot(client: &Arc<dyn ChatStream>, req: ChatRequest) -> Result<String> {
    let mut rx = client.chat_stream(req)?;
    let mut text = String::new();
    while let Some(ev) = rx.recv().await {
        match ev {
            LlmEvent::TextDelta(t) => text.push_str(&t),
            LlmEvent::Completed { text: t, .. } => {
                if !t.is_empty() {
                    text = t;
                }
                break;
            }
            LlmEvent::Error(e) => return Err(anyhow::anyhow!(e)),
            _ => {}
        }
    }
    Ok(text)
}
