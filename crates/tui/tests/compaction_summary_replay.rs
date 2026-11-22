//! Compaction-summary visibility across every view rebuild that starts from
//! the folded transcript: resume replay, mid-run `TranscriptReset` rebuild,
//! and the final `Compaction` event that follows it (no duplicate twin).

use std::sync::Arc;

use opencoder_core::{ContentBlock, Message};
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

/// Text of every plain `User` echo block, in transcript order.
fn user_texts(chat: &ChatView) -> Vec<String> {
    chat.blocks
        .iter()
        .filter_map(|b| match b {
            ChatBlock::User { rendered } => Some(
                rendered
                    .iter()
                    .map(|line| {
                        line.spans
                            .iter()
                            .map(|span| span.content.to_string())
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => None,
        })
        .collect()
}

fn user_count(chat: &ChatView) -> usize {
    user_texts(chat).len()
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
    // UI order: summary head first, retained tail turn below it.
    assert!(
        compaction_index(&chat) < user_index(&chat),
        "summary block renders at the transcript head, matching the live finalize order"
    );
}

/// Mid-run compaction: `TranscriptReset` rebuild renders the summary twin
/// alone (no in-flight `User` echo), the trailing `Compaction(summary)`
/// event finalizes onto it — exactly one block, summary text present.
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
    let msgs = vec![summary_message(summary)];
    rebuild_after_reset(&mut chat, &msgs, &store, "s-midrun").await;
    assert_eq!(compaction_count(&chat), 1, "rebuild renders the twin");
    assert_eq!(user_count(&chat), 0, "compaction drops the in-flight echo");
    assert!(chat.pending_turn_echo.is_none());
    assert_eq!(chat.turn_block_start, chat.blocks.len());
    assert!(compaction_text(&chat).contains("folded"));

    chat.apply(&SessionEvent::Compaction(summary.into()));
    assert_eq!(
        compaction_count(&chat),
        1,
        "final event must not stack a duplicate twin"
    );
    assert_eq!(user_count(&chat), 0);
    assert!(compaction_text(&chat).contains("folded"));
    let _ = dir;
}

/// Mid-run compaction records the in-flight prompt before folding, so the
/// folded transcript's retained tail contains it. The reset rebuild must
/// drop that copy and the remembered echo: the view continues from the
/// Compaction summary alone, with no `User` boundary re-pushed.
#[tokio::test]
async fn transcript_reset_drops_retained_in_flight_echo() {
    let (dir, store) = fresh().await;
    store
        .create_session(&SessionMeta {
            id: "s-midrun-echo".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let mut chat = ChatView {
        pending_turn_echo: Some("the running prompt".into()),
        ..Default::default()
    };

    let msgs = vec![
        summary_message("folded: earlier work"),
        Message::user("u2", "the running prompt"),
    ];
    rebuild_after_reset(&mut chat, &msgs, &store, "s-midrun-echo").await;

    assert_eq!(user_count(&chat), 0, "retained tail copy must be dropped");
    assert_eq!(compaction_count(&chat), 1);
    assert!(chat.pending_turn_echo.is_none());
    let _ = dir;
}

/// Compaction retains `tail_turns` history turns above the summary, but the
/// in-flight prompt is neither retained above the summary nor re-pushed
/// below it.
#[tokio::test]
async fn transcript_reset_keeps_prior_tail_turns_but_drops_in_flight_echo() {
    let (dir, store) = fresh().await;
    store
        .create_session(&SessionMeta {
            id: "s-midrun-prior".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let mut chat = ChatView {
        pending_turn_echo: Some("the running prompt".into()),
        ..Default::default()
    };

    let msgs = vec![
        summary_message("folded: earlier work"),
        Message::user("u1", "prior prompt"),
        Message::user("u2", "the running prompt"),
    ];
    rebuild_after_reset(&mut chat, &msgs, &store, "s-midrun-prior").await;

    assert_eq!(user_count(&chat), 1, "prior retained turn still renders");
    let texts = user_texts(&chat);
    assert!(texts[0].contains("prior prompt"));
    assert!(
        !texts.iter().any(|t| t.contains("the running prompt")),
        "in-flight prompt must not be retained above or re-pushed below"
    );
    assert!(
        compaction_index(&chat) < user_index(&chat),
        "summary renders at its persisted head position, above the retained prior turn"
    );
    assert_eq!(compaction_count(&chat), 1);
    assert!(chat.pending_turn_echo.is_none());
    let _ = dir;
}

/// A compacted transcript with no retained tail `User` message must not
/// re-push the remembered echo below the summary.
#[tokio::test]
async fn transcript_reset_without_tail_prompt_does_not_push_echo() {
    let (dir, store) = fresh().await;
    store
        .create_session(&SessionMeta {
            id: "s-midrun-no-tail".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let mut chat = ChatView {
        pending_turn_echo: Some("the running prompt".into()),
        ..Default::default()
    };

    let msgs = vec![summary_message("folded: earlier work")];
    rebuild_after_reset(&mut chat, &msgs, &store, "s-midrun-no-tail").await;

    assert_eq!(user_count(&chat), 0);
    assert_eq!(compaction_count(&chat), 1);
    assert!(chat.pending_turn_echo.is_none());
    let _ = dir;
}

/// Clear-context rebuilds keep the running turn's user boundary: the
/// remembered echo is re-pushed below the rebuilt blocks. The synthetic seed
/// is continuity context carried across the clear, not an in-flight prompt.
#[tokio::test]
async fn clear_context_reset_keeps_in_flight_echo() {
    let (dir, store) = fresh().await;
    store
        .create_session(&SessionMeta {
            id: "s-clear-echo".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let mut chat = ChatView {
        pending_turn_echo: Some("the running prompt".into()),
        ..Default::default()
    };

    let mut seed = Message::user("seed", "carried context");
    seed.synthetic = true;
    rebuild_after_reset(&mut chat, &[seed], &store, "s-clear-echo").await;

    assert_eq!(user_count(&chat), 1, "remembered echo re-pushed");
    assert!(user_texts(&chat)[0].contains("the running prompt"));
    assert_eq!(compaction_count(&chat), 0);
    assert_eq!(
        chat.pending_turn_echo.as_deref(),
        Some("the running prompt")
    );
    let _ = dir;
}

/// `/task` switch replays the persisted folded transcript
/// `[summary, tail, continued run]`; the Compaction block must render at the
/// summary's persisted head position so the view matches what execution time
/// showed when compaction completed (continued turns below the marker).
#[tokio::test]
async fn task_switch_replay_matches_execution_order() {
    let (dir, store) = fresh().await;
    store
        .create_session(&SessionMeta {
            id: "s-switch".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    let mut reply = Message::assistant("a1");
    reply.blocks.push(ContentBlock::text("kept going"));
    let msgs = vec![
        summary_message("folded: earlier work"),
        Message::user("u2", "continued question"),
        reply,
    ];
    let chat = replay_into_chat("act", &msgs, &store, "s-switch", 0).await;
    let _ = dir;

    assert_eq!(compaction_count(&chat), 1);
    assert!(matches!(chat.blocks.first(), Some(ChatBlock::Compaction { .. })));
    assert!(
        compaction_index(&chat) < user_index(&chat),
        "continued turns render below the head-position summary block"
    );
}
