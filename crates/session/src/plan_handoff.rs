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
use opencoder_core::ContentBlock;
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
/// The plan text comes exclusively from the phase-bounded
/// `SessionState::plan_snapshot` (captured by `record` while the plan agent
/// answers, rescued by compaction). Returns
/// `Some(display_text)` when a reset happened (the display text is the plan +
/// optional extra, suitable for rendering in the UI, WITHOUT the LLM directive
/// prefix); returns `None` when no plan could be found (the caller should leave
/// the transcript untouched).
///
/// The durable store is NOT modified: it stays append-only so the full raw
/// transcript is preserved for audit, exactly like compaction.
pub fn handoff(session: &mut SessionState, extra: &str) -> Option<String> {
    // Phase-bounded single source: the snapshot captured while the plan agent
    // actually produced assistant text in THIS phase — written by
    // `SessionState::record` on every plan-mode assistant turn, rescued by
    // compaction before folding the plan into the summary head, cleared on
    // phase reset / handoff. The old "last non-empty assistant text in the
    // whole transcript" scan is deliberately gone: it had no phase boundary,
    // so a plan requirement whose turn failed or was cancelled BEFORE the
    // LLM produced anything extracted the *earlier act-phase answer* instead,
    // wrapped it as a "plan", collapsed the transcript and persisted an
    // irreversible `handoff_seq` boundary — perceived as "Shift+Tab wiped
    // all context and kept no plan". An empty snapshot means the phase
    // produced no plan: return `None`, keep the context untouched.
    let plan = session.plan_snapshot.clone()?;

    // Total store messages that predate the handoff (the plan-mode history to
    // trim on resume). The in-memory head may hold a synthetic message absent
    // from the store — a prior compaction summary (summary_seq) or a prior
    // plan->act handoff / clear-context marker (handoff_seq) — in which case
    // the store count is `skip + len - 1`. Mirrors SessionState::store_message_count.
    let store_msg_count = if let Some(skip) = session.summary_seq {
        skip as usize + session.messages.len().saturating_sub(1)
    } else if let Some(skip) = session.handoff_seq {
        skip as usize + session.messages.len().saturating_sub(1)
    } else {
        session.messages.len()
    };

    // Display text for the UI plan card: the plan plus any text the user
    // left in the plan-mode input box. This is what the user sees — NOT the
    // LLM directive prefix.
    let mut display = plan.clone();
    let extra = extra.trim();
    if !extra.is_empty() {
        display.push_str("\n\n");
        display.push_str(extra);
    }

    // Preserve recent images from the plan-mode transcript being discarded so
    // they travel with the handoff instruction into act mode. Attaching them to
    // the single handoff message keeps `messages.len() == 1`, so the store
    // accounting in `after_handoff` is unchanged.
    let preserved_images = crate::compaction::collect_head_images(&session.messages);
    let mut msg = handoff_message(&display);
    for url in &preserved_images {
        msg.blocks.push(ContentBlock::Image {
            url: url.clone(),
            detail: None,
        });
    }
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

/// Extract the newest assistant text from the LIVE transcript. Used ONLY by
/// compaction (plan mode) to capture the `plan_snapshot` before folding the
/// plan into the user-role summary head — `handoff` itself never scans the
/// transcript (phase-bounded snapshot only, see [`handoff`]), so a failed or
/// cancelled plan turn can no longer hand a stale act-phase answer forward.
pub fn final_plan_text(messages: &[Message]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant && !m.text().trim().is_empty())
        .map(|m| m.text())
}
