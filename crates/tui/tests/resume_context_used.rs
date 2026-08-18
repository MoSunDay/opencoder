//! Regression test: after resuming a session, the status-bar ctx% indicator
//! must reflect the real transcript length, not stay at zero.
//!
//! Root cause: `replay_into_chat` (the resume path) rebuilt only the display
//! `ChatBlock`s and never touched `ChatView::context_used`, leaving it at its
//! default of 0 — so the indicator showed only the system-prompt token count
//! regardless of how much history existed.

use std::sync::Arc;

use opencoder_core::{ContentBlock, Message, Role};
use opencoder_llm::estimate_messages_for_display;
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
        summary_images: vec![],
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
        requirement: None,
        plan_snapshot: None,
        plan_input_count: 0,
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
        estimate_messages_for_display(&messages) as u64,
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

    let short = vec![Message::user("u1", "hi"), assistant("a1", "hello there")];
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

// --- child view context_used (Fix 1: reconstruct_child_view / replay_messages) ---

use opencoder_store::{SubagentStatus, SubagentTaskRecord};
use opencoder_tui::chat::ChatBlock;

fn assistant_with_task(id: &str, task_id: &str) -> Message {
    Message {
        id: id.into(),
        role: Role::Assistant,
        blocks: vec![
            ContentBlock::Text {
                text: "delegating".into(),
            },
            ContentBlock::ToolUse {
                id: task_id.into(),
                name: "task".into(),
                input: serde_json::json!({"prompt": "explore"}),
            },
        ],
        model: None,
        agent: None,
        usage: Default::default(),
        created_at: 0,
        synthetic: false,
    }
}

#[tokio::test]
async fn child_view_context_used_is_nonzero() {
    let (_dir, store) = fresh().await;
    make_session(&store, "parent").await;
    make_session(&store, "child-1").await;
    let store_arc: Arc<dyn Store> = store.clone();

    let child_msgs = vec![
        Message::user("cu1", "explore the codebase thoroughly"),
        assistant("ca1", "found 3 files implementing the drain loop"),
    ];
    for msg in &child_msgs {
        store.append_message("child-1", msg).await.unwrap();
    }

    let parent_msgs = vec![
        Message::user("u1", "please explore"),
        assistant_with_task("a1", "task-1"),
    ];

    store
        .create_subagent_task(&SubagentTaskRecord {
            task_id: "task-1".into(),
            parent_session_id: "parent".into(),
            child_session_id: "child-1".into(),
            parent_message_id: Some("a1".into()),
            agent: "explore".into(),
            prompt: "explore".into(),
            result: Some("done".into()),
            status: SubagentStatus::Completed,
            ok: Some(true),
            started_at: 0,
            completed_at: Some(1),
        })
        .await
        .unwrap();

    let chat = replay_into_chat("act", &parent_msgs, &store_arc, "parent").await;

    let sub = chat
        .blocks
        .iter()
        .find_map(|b| match b {
            ChatBlock::Subagent { view, .. } => Some(view),
            _ => None,
        })
        .expect("expected a Subagent block");

    assert!(
        sub.context_used > 0,
        "child view context_used must be non-zero, got {}",
        sub.context_used
    );
    assert_eq!(
        sub.context_used,
        estimate_messages_for_display(&child_msgs) as u64,
        "child view context_used must match display estimate of child messages"
    );
}

#[tokio::test]
async fn replay_messages_context_used_is_nonzero() {
    use opencoder_tui::session_ui::replay_messages;

    let msgs = vec![
        Message::user("u1", "hello world from the test suite"),
        assistant("a1", "hi there, this is a response"),
    ];
    let view = replay_messages("act", &msgs);
    assert!(
        view.context_used > 0,
        "replay_messages must set context_used, got {}",
        view.context_used
    );
    assert_eq!(
        view.context_used,
        estimate_messages_for_display(&msgs) as u64
    );
}
