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
/// repackaged as a single synthetic user instruction.
///
/// The "final plan" is the last assistant message carrying non-empty text —
/// per the plan agent prompt that is where the actionable plan lives. Returns
/// `true` when a reset happened, `false` when no plan could be found (the
/// caller should then leave the transcript untouched and fall back to current
/// behavior).
///
/// The durable store is NOT modified: it stays append-only so the full raw
/// transcript is preserved for audit, exactly like compaction.
pub fn handoff(session: &mut SessionState) -> bool {
    let Some(plan) = final_plan_text(&session.messages) else {
        return false;
    };

    let mut msg = Message::user(new_id(), format!("{HANDOFF_PREFIX}{plan}"));
    msg.synthetic = true;
    session.messages = vec![msg];
    true
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
