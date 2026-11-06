//! Compaction-summary visibility across every view rebuild that starts from
//! the folded transcript: resume replay, mid-run `TranscriptReset` rebuild,
//! and the final `Compaction` event that follows it (no duplicate twin).

use std::sync::Arc;

use opencoder_core::Message;
use opencoder_session::SessionEvent;
use opencoder_store::{SessionMeta, Store, TursoStore};
use opencoder_tui::chat::{ChatBlock, ChatView};
use opencoder_tui::session_ui::{rebuild_after_reset, replay_into_chat};

async fn fresh() -> (tempfile::TempDir, Arc<dyn Store>) {
    let dir = tempfile::tempdir().unwrap();
    let store = TursoStore::open(dir.path().join("test.db")).await.unwrap();
    (dir, Arc::from(store) as Arc<dyn Store>)
}

/// Shape mirrors `opencoder_session::compaction::compaction_message`: the
/// synthetic in-memory-only head message of a compacted transcript.
fn summary_message(summary: &str) -> Message {
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

fn compaction_count(chat: &ChatView) -> usize {
    chat.blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::Compaction { .. }))
        .count()
}

/// Index of the (single) Compaction block — for order assertions.
fn compaction_index(chat: &ChatView) -> Option<usize> {
    chat.blocks
        .iter()
        .position(|b| matches!(b, ChatBlock::Compaction { .. }))
}

/// Index of the first plain `User` echo block.
fn user_index(chat: &ChatView) -> Option<usize> {
    chat.blocks
        .iter()
        .position(|b| matches!(b, ChatBlock::User { .. }))
}

fn compaction_text(chat: &ChatView) -> String {
    chat.blocks
        .iter()
        .find_map(|b| match b {
            ChatBlock::Compaction { text, .. } => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// Resume: the folded transcript's summary head must replay into a
/// Compaction block — before the fix the synthetic head was skipped and the
/// compressed history vanished on every restart / task switch.
#[tokio::test]
async fn resume_replay_renders_summary_head() {
    let (dir, store) = fresh().await;
    store
        .create_session(&SessionMeta {
            id: "s-resume".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let msgs = vec![
        summary_message("turn one built a tool\nturn two fixed a bug"),
        Message::user("u2", "continue"),
    ];
    let chat = replay_into_chat("act", &msgs, &store, "s-resume", 0).await;
    let _ = dir;
    assert_eq!(compaction_count(&chat), 1);
    assert!(compaction_text(&chat).contains("turn two fixed a bug"));
    // UI order: retained tail turn first, folded-history marker after it.
    assert!(
        compaction_index(&chat) > user_index(&chat),
        "summary block must render below the retained tail turns"
    );
}

/// Mid-run compaction: `TranscriptReset` rebuild renders the twin (after
/// the retained tail), the trailing `Compaction(summary)` event finalizes
/// onto it — exactly one block, summary text present, with the in-flight
/// echo re-pushed below the twin (`tail + summary + continued execution`).
#[tokio::test]
async fn transcript_reset_then_compaction_event_renders_single_block() {
    let (dir, store) = fresh().await;
    store
        .create_session(&SessionMeta {
            id: "s-midrun".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let mut chat = ChatView::default();
    chat.apply(&SessionEvent::CompactionDelta("streamed chunk".into()));
    chat.pending_turn_echo = Some("the running prompt".into());

    let summary = "folded: built, tested, shipped";
    let msgs = vec![summary_message(summary), Message::user("u2", "next")];
    rebuild_after_reset(&mut chat, &msgs, &store, "s-midrun").await;
    assert_eq!(compaction_count(&chat), 1, "rebuild renders the twin");
    assert!(compaction_text(&chat).contains("folded"));

    chat.apply(&SessionEvent::Compaction(summary.into()));
    assert_eq!(
        compaction_count(&chat),
        1,
        "final event must not stack a duplicate twin"
    );
    assert!(compaction_text(&chat).contains("folded"));
    // Order contract: retained tail turn < summary marker < in-flight echo
    // (`tail + summary + continued execution`); the echo is re-pushed by
    // `rebuild_after_reset` AFTER the replayed blocks.
    let tail_user = user_index(&chat).expect("retained tail echo");
    let summary_idx = compaction_index(&chat).expect("summary block");
    let echo_user = chat
        .blocks
        .iter()
        .rposition(|b| matches!(b, ChatBlock::User { .. }))
        .expect("in-flight echo boundary");
    assert!(summary_idx > tail_user, "summary below the retained tail");
    assert!(echo_user > summary_idx, "continued run below the summary");
    let _ = dir;
}
