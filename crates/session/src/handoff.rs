//! Transcript handoff: collapse the live transcript into a single synthetic
//! directive message.
//!
//! Used by the autopilot ACT phase: the read-only exploration pass (sandbox
//! agent + review skill) produces a brief, then the transcript is reset so
//! the act agent starts from only that brief — not the full exploration
//! chatter (subagent noise, clarifying Q&A).
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

/// Instruction prepended to the extracted brief. Worded as a user directive so
/// the act agent treats the brief as the task to execute rather than re-exploring.
pub(crate) const HANDOFF_PREFIX: &str = "\
Exploration phase complete. The brief below was produced in a read-only pass. \
Execute it now: make the described changes, run builds/tests, and verify. \
Do not re-explore; proceed directly with implementation.\n\n";

/// Reset the transcript for an execution handoff: keep only the newest real
/// assistant brief, repackaged as a single synthetic user instruction. `extra`
/// (any text the user submitted alongside the switch) is appended when
/// non-empty, so it is submitted as part of the same directive.
///
/// The brief is the newest NON-synthetic assistant text in the live
/// transcript. Synthetic messages (compaction summaries, earlier handoff
/// directives) are skipped: only real agent output qualifies — a summarizer's
/// paraphrase must never be mistaken for the brief. No assistant text at all
/// means the exploration pass produced nothing: return `None` and keep the
/// context untouched.
pub fn reset_to_directive(session: &mut SessionState, extra: &str) -> Option<String> {
    let brief = newest_work_text(&session.messages)?;

    // Total store messages that predate the handoff (the history to trim on
    // resume). The in-memory head may hold a synthetic message absent from the
    // store — a prior compaction summary (summary_seq) or a prior handoff /
    // clear-context marker (handoff_seq) — in which case the store count is
    // `skip + len - 1`. Mirrors SessionState::store_message_count.
    let store_msg_count = if let Some(skip) = session.summary_seq {
        skip as usize + session.messages.len().saturating_sub(1)
    } else if let Some(skip) = session.handoff_seq {
        skip as usize + session.messages.len().saturating_sub(1)
    } else {
        session.messages.len()
    };

    // Display text for the UI handoff card: the brief plus any text the user
    // submitted alongside. This is what the user sees — NOT the LLM directive
    // prefix.
    let mut display = brief;
    let extra = extra.trim();
    if !extra.is_empty() {
        display.push_str("\n\n");
        display.push_str(extra);
    }

    // Preserve recent images from the discarded transcript so they travel
    // with the handoff instruction. Attaching them to the single handoff
    // message keeps `messages.len() == 1`, so the store accounting in
    // `after_handoff` is unchanged.
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

/// Build the synthetic handoff directive message from the display text
/// (brief + optional extra). The LLM body is the directive prefix followed by
/// the display text. Exposed so `resume` can reconstruct the exact same
/// message for legacy persisted boundaries without duplicating the prefix.
pub fn handoff_message(display: &str) -> Message {
    let body = format!("{HANDOFF_PREFIX}{display}");
    let mut msg = Message::user(new_id(), body);
    msg.synthetic = true;
    msg
}

/// Newest non-synthetic, non-empty assistant text in the transcript — the
/// single extraction source for [`reset_to_directive`].
pub fn newest_work_text(messages: &[Message]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant && !m.synthetic && !m.text().trim().is_empty())
        .map(|m| m.text())
}

/// Newest non-empty assistant text in the transcript (synthetic included).
/// Used by the clear-context seed path: the preserved last say travels into
/// the fresh transcript as neutral prior context.
pub fn last_assistant_text(messages: &[Message]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant && !m.text().trim().is_empty())
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
    fn newest_work_text_picks_newest_real_answer() {
        let msgs = vec![
            Message::user("u1", "explore X"),
            assistant("a1", "sandbox", "older answer"),
            assistant("a2", "sandbox", "newest answer"),
        ];
        assert_eq!(newest_work_text(&msgs).as_deref(), Some("newest answer"));
    }

    #[test]
    fn newest_work_text_skips_synthetic_and_empty() {
        // The newest text is synthetic (a compaction summary): skipped in
        // favour of the older real answer.
        let synthetic_newest = vec![
            Message::user("u1", "task"),
            assistant("a2", "sandbox", "real answer"),
            synthetic_assistant("s1", "act", "summary head"),
        ];
        assert_eq!(
            newest_work_text(&synthetic_newest).as_deref(),
            Some("real answer")
        );

        // The newest real text is whitespace-only: skipped.
        let empty_newest = vec![
            Message::user("u1", "task"),
            assistant("a2", "sandbox", "real answer"),
            assistant("a3", "sandbox", "   "),
        ];
        assert_eq!(
            newest_work_text(&empty_newest).as_deref(),
            Some("real answer")
        );

        // No assistant message at all.
        assert_eq!(newest_work_text(&[Message::user("u1", "hi")]), None);
    }

    #[test]
    fn last_assistant_text_includes_synthetic() {
        let msgs = vec![
            Message::user("u1", "hi"),
            assistant("a1", "act", "task done"),
            synthetic_assistant("s1", "act", "summary"),
        ];
        assert_eq!(
            last_assistant_text(&msgs).as_deref(),
            Some("summary"),
            "seed path preserves ANY last say, synthetic included"
        );
    }
}
