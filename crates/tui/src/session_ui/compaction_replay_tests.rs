//! Replay-side compaction-summary rendering: the synthetic summary message
//! of a folded transcript must survive replay as the same collapsible
//! Compaction block the live path finalizes — resume, task switch, remote
//! re-attach and mid-run `TranscriptReset` rebuilds all pass through here.
//! UI order: retained tail turns FIRST, summary block AFTER them (the
//! summary-first message list is an LLM-context concern only).

use super::replay::replay_messages;
use crate::chat::ChatBlock;
use opencoder_core::Message;

fn summary_message(summary: &str) -> Message {
    // Shape mirrors `opencoder_session::compaction::compaction_message`.
    let mut m = Message::user(
        "u-summary",
        format!(
            "{}\n{summary}",
            opencoder_session::compaction::SUMMARY_PREFIX
        ),
    );
    m.synthetic = true;
    m
}

fn compaction_blocks(chat: &crate::chat::ChatView) -> Vec<&ChatBlock> {
    chat.blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::Compaction { .. }))
        .collect()
}

/// The summary renders as ONE collapsed Compaction block with the marker
/// prefix stripped — not as a `user:` echo, not dropped — and it renders
/// AFTER the retained tail turn: the folded-history marker sits between
/// the kept turns and whatever continues below them.
#[test]
fn replay_defers_summary_block_below_retained_tail() {
    let msgs = vec![
        summary_message("folded turn one\nfolded turn two"),
        Message::user("u2", "next question"),
    ];
    let chat = replay_messages("act", &msgs);

    let blocks = compaction_blocks(&chat);
    assert_eq!(blocks.len(), 1, "exactly one compaction block");
    match blocks[0] {
        ChatBlock::Compaction {
            text,
            collapsed,
            streaming,
        } => {
            assert!(text.contains("folded turn one"));
            assert!(text.contains("folded turn two"));
            assert!(!text.contains("Conversation summary so far"));
            assert!(collapsed, "replay twin starts collapsed like finalize");
            assert!(!streaming);
        }
        _ => unreachable!("filtered above"),
    }
    // Retained tail first, folded-history marker after it.
    assert!(matches!(
        chat.blocks.first(),
        Some(ChatBlock::User { .. })
    ));
    assert!(matches!(
        chat.blocks.last(),
        Some(ChatBlock::Compaction { .. })
    ));
}

/// The synthetic summary is internal: it must not become `first_prompt`
/// (title/echo bookkeeping) even though it is the transcript's first text.
#[test]
fn replay_summary_does_not_pollute_first_prompt() {
    let msgs = vec![
        summary_message("folded"),
        Message::user("u2", "real prompt"),
    ];
    let chat = replay_messages("act", &msgs);
    assert_eq!(chat.first_prompt.as_deref(), Some("real prompt"));
}

/// A real (non-synthetic) user message that merely mentions the marker is
/// genuine input — rendered as a user block, never swallowed as a summary.
#[test]
fn replay_non_synthetic_marker_text_stays_user_input() {
    let m = Message::user(
        "u1",
        format!("{}\nquoted", opencoder_session::compaction::SUMMARY_PREFIX),
    );
    let chat = replay_messages("act", &[m]);
    assert!(compaction_blocks(&chat).is_empty());
    assert!(matches!(chat.blocks.first(), Some(ChatBlock::User { .. })));
}

/// Other synthetic user messages (plan->act handoff / clear-context seeds)
/// stay skipped: only the summary marker renders.
#[test]
fn replay_still_skips_other_synthetic_messages() {
    let mut seed = Message::user("u-seed", "internal directive");
    seed.synthetic = true;
    let chat = replay_messages("act", &[seed]);
    assert!(chat.blocks.is_empty());
}
