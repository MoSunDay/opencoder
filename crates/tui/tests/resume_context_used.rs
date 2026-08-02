//! Regression test: after resuming a session, the status-bar ctx% indicator
//! must reflect the real transcript length, not stay at zero.
//!
//! Root cause: `replay_into_chat` (the resume path) rebuilt only the display
//! `ChatBlock`s and never touched `ChatView::context_used`, leaving it at its
//! default of 0 — so the indicator showed only the system-prompt token count
//! regardless of how much history existed.

use std::sync::Arc;

use opencoder_core::{ContentBlock, Message, Role};
use opencoder_llm::estimate_messages;
use opencoder_store::{LibsqlStore, SessionMeta, Store};
use opencoder_tui::session_ui::replay_into_chat;
use tempfile::TempDir;

async fn fresh() -> (TempDir, Arc<LibsqlStore>) {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();
    (dir, Arc::new(store))
}

async fn make_session(store: &LibsqlStore, id: &str) {
    let meta = SessionMeta {
        id: id.to_string(),
        title: Some(format!("title-{id}")),
        agent: Some("act".into()),
        model: Some("m".into()),
        workdir_hash: None,
        created_at: 1000,
        updated_at: 1000,
        summary: None,
        summary_seq: None,
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
    };
    store.create_session(&meta).await.unwrap();
}

fn assistant(id: &str, text: &str) -> Message {
    Message {
        id: id.into(),
        role: Role::Assistant,
        blocks: vec![ContentBlock::text(text)],
        model: None,
        agent: None,
        usage: Default::default(),
        created_at: 0,
        synthetic: false,
    }
}

#[tokio::test]
async fn resume_context_used_matches_transcript_estimate() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s1").await;
    let store_arc: Arc<dyn Store> = store.clone();

    // A realistic turn pair: user prompt + assistant reply.
    let messages = vec![
        Message::user("u1", "please explain how the drain loop works in detail"),
        assistant("a1", "the drain loop promotes steer at each turn boundary and consumes exactly one queue item when idle"),
    ];

    let chat = replay_into_chat("act", &messages, &store_arc, "s1").await;

    assert!(
        chat.context_used > 0,
        "context_used must be non-zero after resume, got {}",
        chat.context_used
    );
    assert_eq!(
        chat.context_used,
        estimate_messages(&messages) as u64,
        "context_used must equal the transcript token estimate"
    );
}

#[tokio::test]
async fn resume_context_used_empty_when_no_messages() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s2").await;
    let store_arc: Arc<dyn Store> = store.clone();

    let chat = replay_into_chat("act", &[], &store_arc, "s2").await;

    assert_eq!(
        chat.context_used, 0,
        "empty transcript must yield zero context_used"
    );
}

#[tokio::test]
async fn resume_context_used_grows_with_more_messages() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s3").await;
    let store_arc: Arc<dyn Store> = store.clone();

    let short = vec![
        Message::user("u1", "hi"),
        assistant("a1", "hello there"),
    ];
    let long = vec![
        Message::user("u1", "hi"),
        assistant("a1", "hello there"),
        Message::user("u2", "can you now write a long explanation about token estimation and compaction thresholds"),
        assistant("a2", "token estimation uses a chars-per-token heuristic so compaction can fire before any usage is reported by the model"),
    ];

    let chat_short = replay_into_chat("act", &short, &store_arc, "s3").await;
    let chat_long = replay_into_chat("act", &long, &store_arc, "s3").await;

    assert!(
        chat_long.context_used > chat_short.context_used,
        "longer transcript must have larger context_used ({} > {})",
        chat_long.context_used,
        chat_short.context_used
    );
}
