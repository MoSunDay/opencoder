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
/// The plan text comes first from the phase-bounded
/// `SessionState::plan_snapshot` (captured by `record` while the plan agent
/// answers, rescued by compaction). When the snapshot is missing (e.g. the
/// phase state was reset by a manual switch back to plan mode, or a legacy
/// session predates the snapshot column), it falls back to the newest
/// assistant text tagged `agent == "plan"` in the live transcript — a
/// message-level phase boundary: act-mode answers are tagged `"act"` and can
/// never be mistaken for a plan. Returns
/// `Some(display_text)` when a reset happened (the display text is the plan +
/// optional extra, suitable for rendering in the UI, WITHOUT the LLM directive
/// prefix); returns `None` when no plan could be found (the caller should leave
/// the transcript untouched).
///
/// The durable store is NOT modified: it stays append-only so the full raw
/// transcript is preserved for audit, exactly like compaction.
pub fn handoff(session: &mut SessionState, extra: &str) -> Option<String> {
    // Phase-bounded primary source: the snapshot captured while the plan
    // agent actually produced assistant text in THIS phase — written by
    // `SessionState::record` on every plan-mode assistant turn, rescued by
    // compaction before folding the plan into the summary head, retired when
    // a new requirement is recorded (`maybe_tag_plan_prompt`) or consumed by
    // handoff. When the snapshot is absent the message-level
    // fallback (`newest_plan_agent_text`) scans the transcript for the
    // newest assistant text TAGGED as plan output. The old untagged
    // "last non-empty assistant text in the whole transcript" scan is
    // deliberately gone: it had no phase boundary, so a plan requirement
    // whose turn failed or was cancelled BEFORE the LLM produced anything
    // extracted the *earlier act-phase answer* instead, wrapped it as a
    // "plan", collapsed the transcript and persisted an irreversible
    // `handoff_seq` boundary — perceived as "Shift+Tab wiped all context
    // and kept no plan". The tag filter keeps that anti-fabrication
    // guarantee: act answers are tagged `"act"`, never `"plan"`. Neither
    // source producing a plan means the phase produced no plan: return
    // `None`, keep the context untouched.
    let plan = session
        .plan_snapshot
        .clone()
        .or_else(|| newest_plan_agent_text(&session.messages))?;

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

/// Extract the newest assistant text from the LIVE transcript. Used by
/// compaction (plan mode) to capture the `plan_snapshot` before folding the
/// plan into the user-role summary head.
pub fn final_plan_text(messages: &[Message]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant && !m.text().trim().is_empty())
        .map(|m| m.text())
}

/// Newest assistant text TAGGED as plan output in the live transcript —
/// the message-level phase boundary for [`handoff`]'s snapshot fallback and
/// for `resume`'s legacy plan-phase backfill. The runner writes
/// `Message::agent` on every assistant turn, so an act-mode answer is tagged
/// `"act"` and can never be recovered as a plan (anti-fabrication, the
/// `ecce7b0` guarantee). Synthetic messages (compaction summaries, handoff
/// directives) are skipped: only real plan-agent output qualifies.
pub fn newest_plan_agent_text(messages: &[Message]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|m| {
            m.role == Role::Assistant
                && !m.synthetic
                && m.agent.as_deref() == Some("plan")
                && !m.text().trim().is_empty()
        })
        .map(|m| m.text())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assistant(id: &str, agent: &str, text: &str) -> Message {
        let mut m = Message::assistant(id);
        m.blocks.push(ContentBlock::text(text));
        m.agent = Some(agent.into());
        m
    }

    fn synthetic_assistant(id: &str, agent: &str, text: &str) -> Message {
        let mut m = assistant(id, agent, text);
        m.synthetic = true;
        m
    }

    #[test]
    fn newest_plan_agent_text_picks_newest_plan_tagged_answer() {
        let msgs = vec![
            Message::user("u1", "do task X"),
            assistant("a1", "act", "task done"),
            Message::user("u2", "plan feature Y"),
            assistant("a2", "plan", "## Plan\n1. do X"),
            Message::user("u3", "plan feature Z"),
            assistant("a3", "plan", "## Plan\n1. do X\n2. do Z"),
        ];
        assert_eq!(
            newest_plan_agent_text(&msgs).as_deref(),
            Some("## Plan\n1. do X\n2. do Z")
        );
    }

    #[test]
    fn newest_plan_agent_text_skips_act_synthetic_and_empty() {
        // Only an act-mode answer exists: nothing qualifies as a plan.
        let act_only = vec![
            Message::user("u1", "do task X"),
            assistant("a1", "act", "task done"),
        ];
        assert_eq!(newest_plan_agent_text(&act_only), None);

        // The newest plan-tagged text is synthetic (a handoff directive):
        // skipped in favour of the older real plan answer.
        let synthetic_newest = vec![
            Message::user("u1", "plan feature Y"),
            assistant("a2", "plan", "## Plan\n1. do X"),
            synthetic_assistant("s1", "plan", "Planning phase complete."),
        ];
        assert_eq!(
            newest_plan_agent_text(&synthetic_newest).as_deref(),
            Some("## Plan\n1. do X")
        );

        // The newest plan-tagged text is whitespace-only: skipped.
        let empty_newest = vec![
            Message::user("u1", "plan feature Y"),
            assistant("a2", "plan", "## Plan\n1. do X"),
            assistant("a3", "plan", "   "),
        ];
        assert_eq!(
            newest_plan_agent_text(&empty_newest).as_deref(),
            Some("## Plan\n1. do X")
        );

        // No assistant message at all.
        assert_eq!(newest_plan_agent_text(&[Message::user("u1", "hi")]), None);
    }
}
