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

/// Run up to `retries` isolated one-shot VERIFY calls against a throwaway
/// snapshot of the current transcript. Returns the first parseable verdict, or
/// [`VerifyVerdict::Malformed`] if none parse.
///
/// `session.messages` is read-only here (immutable borrow) — nothing is
/// recorded or persisted. A transient LLM error counts as a malformed attempt
/// and is retried within the budget.
pub async fn verify(session: &SessionState, state: &ApState, retries: u32) -> VerifyVerdict {
    // Build the ephemeral snapshot: system + cloned transcript + goal question.
    let mut snapshot = Vec::with_capacity(session.messages.len() + 2);
    snapshot.push(Message::system(new_id(), verify_system_prompt()));
    snapshot.extend(session.messages.iter().cloned());
    snapshot.push(Message::user(new_id(), verify_user_prompt(&state.goal)));
    let msgs = lower_messages(&snapshot);

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
                Some(true) => return VerifyVerdict::MoreWork,
                Some(false) => return VerifyVerdict::Complete,
                None => continue, // malformed → retry within budget
            },
            Err(_) => continue, // transient error → retry within budget
        }
    }
    VerifyVerdict::Malformed
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
