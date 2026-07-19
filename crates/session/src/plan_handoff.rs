//! Plan→act transcript handoff.
//!
//! When the user switches from plan mode to act mode to execute a finalized
//! plan, the act agent should start from a clean transcript containing only
//! the final plan — not the full read-only planning conversation (exploration
//! chatter, subagent noise, clarifying Q&A).
//!
//! This mirrors compaction's in-memory mutation pattern ([`crate::compaction`]):
//! `session.messages` is replaced directly, the durable store (append-only) is
//! left untouched so the raw transcript stays available for audit / resumption,
//! and a fresh resume reloads the full history — the same trade-off compaction
//! already makes.

use crate::runner::new_id;
use crate::SessionState;
use opencoder_core::{Message, Role};

/// Instruction prepended to the extracted plan. Worded as a user directive so
/// the act agent treats the plan as the task to execute rather than re-planning.
const HANDOFF_PREFIX: &str = "\
Planning phase complete. The plan below was produced in read-only plan mode. \
Execute it now in act mode: make the described changes, run builds/tests, and \
verify. Do not re-plan; proceed directly with implementation.\n\n";

/// Reset the transcript for a plan→act handoff: keep only the final plan,
/// repackaged as a single synthetic user instruction. `extra` (any text the
/// user left in the plan-mode input box) is appended to the plan when
/// non-empty, so it is submitted as part of the same directive.
///
/// The "final plan" is the last assistant message carrying non-empty text —
/// per the plan agent prompt that is where the actionable plan lives. Returns
/// `Some(display_text)` when a reset happened (the display text is the plan +
/// optional extra, suitable for rendering in the UI, WITHOUT the LLM directive
/// prefix); returns `None` when no plan could be found (the caller should leave
/// the transcript untouched).
///
/// The durable store is NOT modified: it stays append-only so the full raw
/// transcript is preserved for audit, exactly like compaction.
pub fn handoff(session: &mut SessionState, extra: &str) -> Option<String> {
    let plan = final_plan_text(&session.messages)?;

    // Total store messages that predate the handoff (the plan-mode history to
    // trim on resume). Mirrors compaction's head_store_msgs accounting: if a
    // prior compaction left a synthetic summary at the head, that message is
    // NOT in the store, so the store count is summary_seq + (messages.len()-1).
    let prior_skip = session.summary_seq.unwrap_or(0) as usize;
    let has_prior_summary = session.summary_seq.is_some();
    let store_msg_count =
        prior_skip + session.messages.len() - if has_prior_summary { 1 } else { 0 };

    // Display text for the UI plan card: the plan plus any text the user
    // left in the plan-mode input box. This is what the user sees — NOT the
    // LLM directive prefix.
    let mut display = plan.clone();
    let extra = extra.trim();
    if !extra.is_empty() {
        display.push_str("\n\n");
        display.push_str(extra);
    }

    let msg = handoff_message(&display);
    session.messages = vec![msg];
    // Record the boundary so resume can reconstruct this focused transcript.
    session.after_handoff(store_msg_count as i64, display.clone());

    Some(display)
}

/// Build the synthetic plan→act handoff instruction message from the display
/// text (plan + optional extra). The LLM body is the directive prefix followed
/// by the display text. Exposed so `resume` can reconstruct the exact same
/// message without duplicating the prefix.
pub fn handoff_message(display: &str) -> Message {
    let body = format!("{HANDOFF_PREFIX}{display}");
    let mut msg = Message::user(new_id(), body);
    msg.synthetic = true;
    msg
}

/// Extract the final plan: the newest assistant message with non-empty text.
/// Newest-first scan so the most recent plan (after any clarifying Q&A) wins;
/// empty / tool-only assistant turns are skipped.
pub fn final_plan_text(messages: &[Message]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant && !m.text().trim().is_empty())
        .map(|m| m.text())
}
