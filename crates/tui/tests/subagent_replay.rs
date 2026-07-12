//! Integration test for subagent block reconstruction on session resume.
//!
//! Verifies that `replay_into_chat` rebuilds `ChatBlock::Subagent` blocks
//! from persisted `subagent_tasks` records — including correct status/summary
//! and a non-empty child view reconstructed from stored events.

use std::sync::Arc;

use opencoder_core::{ContentBlock, Message};
use opencoder_session::SessionEvent;
use opencoder_store::{
    EventKind, LibsqlStore, SessionEventRecord, SessionMeta, Store, SubagentStatus,
    SubagentTaskRecord,
};
use opencoder_tui::chat::ChatBlock;
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
    };
    store.create_session(&meta).await.unwrap();
}

fn child_event(
    session_id: &str,
    ev: &SessionEvent,
    kind: EventKind,
    ts: i64,
) -> SessionEventRecord {
    SessionEventRecord {
        session_id: session_id.to_string(),
        kind,
        // Child events are double-encoded: Value::String(json_string).
        payload: serde_json::Value::String(serde_json::to_string(ev).unwrap()),
        ts,
        seq: None,
    }
}

fn running_task(
    task_id: &str,
    parent: &str,
    child: &str,
    parent_msg_id: &str,
    agent: &str,
    prompt: &str,
    started_at: i64,
) -> SubagentTaskRecord {
    SubagentTaskRecord {
        task_id: task_id.into(),
        parent_session_id: parent.into(),
        child_session_id: child.into(),
        parent_message_id: Some(parent_msg_id.into()),
        agent: agent.into(),
        prompt: prompt.into(),
        result: None,
        status: SubagentStatus::Running,
        ok: None,
        started_at,
        completed_at: None,
    }
}

#[tokio::test]
async fn replay_reconstructs_subagent_blocks_with_status_and_child_view() {
    let (_dir, store) = fresh().await;
    make_session(&store, "parent").await;
    make_session(&store, "child-ok").await;
    make_session(&store, "child-fail").await;

    // Parent messages: user asks, assistant responds (id "a1").
    let user_msg = Message::user("u1", "do the thing");
    let mut asst_msg = Message::assistant("a1");
    asst_msg
        .blocks
        .push(ContentBlock::text("delegating to subagents"));
    store.append_message("parent", &user_msg).await.unwrap();
    store.append_message("parent", &asst_msg).await.unwrap();

    // Subagent task 1: create as Running, then complete successfully.
    store
        .create_subagent_task(&running_task(
            "task-ok",
            "parent",
            "child-ok",
            "a1",
            "explore",
            "find all TODOs",
            1100,
        ))
        .await
        .unwrap();
    store
        .complete_subagent_task("task-ok", "found 5 TODOs", true)
        .await
        .unwrap();

    // Subagent task 2: create as Running, then fail.
    store
        .create_subagent_task(&running_task(
            "task-fail",
            "parent",
            "child-fail",
            "a1",
            "build",
            "run the build",
            1101,
        ))
        .await
        .unwrap();
    store
        .complete_subagent_task("task-fail", "build failed", false)
        .await
        .unwrap();

    // Persist child events so the child views can be reconstructed via
    // event replay (the primary path).
    store
        .append_event(&child_event(
            "child-ok",
            &SessionEvent::TextDelta("child ok text".into()),
            EventKind::TextDelta,
            2000,
        ))
        .await
        .unwrap();
    store
        .append_event(&child_event(
            "child-ok",
            &SessionEvent::Done,
            EventKind::Done,
            2001,
        ))
        .await
        .unwrap();
    store
        .append_event(&child_event(
            "child-fail",
            &SessionEvent::TextDelta("child fail text".into()),
            EventKind::TextDelta,
            2000,
        ))
        .await
        .unwrap();
    store
        .append_event(&child_event(
            "child-fail",
            &SessionEvent::Done,
            EventKind::Done,
            2001,
        ))
        .await
        .unwrap();

    // Rebuild the chat view.
    let messages = store.load_messages("parent").await.unwrap();
    let store_arc: Arc<dyn Store> = store.clone();
    let chat = replay_into_chat("act", &messages, &store_arc, "parent").await;

    // Collect subagent blocks.
    let subagent_blocks: Vec<&ChatBlock> = chat
        .blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::Subagent { .. }))
        .collect();
    assert_eq!(
        subagent_blocks.len(),
        2,
        "should have 2 subagent blocks interleaved after the assistant message"
    );

    // Verify the completed subagent.
    let ok_block = subagent_blocks
        .iter()
        .find(|b| matches!(b, ChatBlock::Subagent { id, .. } if id == "task-ok"))
        .expect("task-ok block should exist");
    if let ChatBlock::Subagent {
        done,
        ok,
        summary,
        view,
        kind,
        prompt,
        child_session_id,
        ..
    } = ok_block
    {
        assert!(*done, "completed subagent should be done");
        assert!(*ok, "completed subagent should be ok");
        assert_eq!(summary, "found 5 TODOs");
        assert_eq!(kind, "explore");
        assert!(prompt.contains("find all TODOs"));
        assert_eq!(child_session_id, "child-ok");
        assert!(
            !view.blocks.is_empty(),
            "child view should have blocks from events"
        );
    }

    // Verify the failed subagent.
    let fail_block = subagent_blocks
        .iter()
        .find(|b| matches!(b, ChatBlock::Subagent { id, .. } if id == "task-fail"))
        .expect("task-fail block should exist");
    if let ChatBlock::Subagent {
        done,
        ok,
        summary,
        view,
        kind,
        ..
    } = fail_block
    {
        assert!(*done, "failed subagent should be done");
        assert!(!*ok, "failed subagent should not be ok");
        assert_eq!(summary, "build failed");
        assert_eq!(kind, "build");
        assert!(
            !view.blocks.is_empty(),
            "child view should have blocks from events"
        );
    }

    // Verify interleaving: subagent blocks should come after the Assistant block.
    let asst_idx = chat
        .blocks
        .iter()
        .position(|b| matches!(b, ChatBlock::Assistant { .. }))
        .expect("should have an Assistant block");
    let first_sub_idx = chat
        .blocks
        .iter()
        .position(|b| matches!(b, ChatBlock::Subagent { .. }))
        .expect("should have a Subagent block");
    assert!(
        first_sub_idx > asst_idx,
        "subagent blocks should be interleaved after the assistant message"
    );
}

#[tokio::test]
async fn replay_falls_back_to_message_replay_when_no_events() {
    let (_dir, store) = fresh().await;
    make_session(&store, "parent").await;
    make_session(&store, "child-noev").await;

    let user_msg = Message::user("u1", "do it");
    let mut asst_msg = Message::assistant("a1");
    asst_msg.blocks.push(ContentBlock::text("delegating"));
    store.append_message("parent", &user_msg).await.unwrap();
    store.append_message("parent", &asst_msg).await.unwrap();

    // Child has messages but no events — tests the fallback path.
    let mut child_asst = Message::assistant("c1");
    child_asst
        .blocks
        .push(ContentBlock::text("child result text"));
    store
        .append_message("child-noev", &child_asst)
        .await
        .unwrap();

    store
        .create_subagent_task(&running_task(
            "task-noev",
            "parent",
            "child-noev",
            "a1",
            "explore",
            "do something",
            1100,
        ))
        .await
        .unwrap();
    store
        .complete_subagent_task("task-noev", "done", true)
        .await
        .unwrap();

    let messages = store.load_messages("parent").await.unwrap();
    let store_arc: Arc<dyn Store> = store.clone();
    let chat = replay_into_chat("act", &messages, &store_arc, "parent").await;

    let sub = chat
        .blocks
        .iter()
        .find(|b| matches!(b, ChatBlock::Subagent { id, .. } if id == "task-noev"))
        .expect("should have the subagent block");

    if let ChatBlock::Subagent { view, .. } = sub {
        assert!(
            !view.blocks.is_empty(),
            "child view should be rebuilt from messages as fallback"
        );
    }
}

#[tokio::test]
async fn replay_shows_interrupted_for_running_subagent() {
    let (_dir, store) = fresh().await;
    make_session(&store, "parent").await;
    make_session(&store, "child-run").await;

    let user_msg = Message::user("u1", "do it");
    let mut asst_msg = Message::assistant("a1");
    asst_msg.blocks.push(ContentBlock::text("delegating"));
    store.append_message("parent", &user_msg).await.unwrap();
    store.append_message("parent", &asst_msg).await.unwrap();

    // A subagent still in Running status (interrupted by process exit).
    // Do NOT call complete_subagent_task — leave it Running.
    store
        .create_subagent_task(&running_task(
            "task-run",
            "parent",
            "child-run",
            "a1",
            "explore",
            "long running task",
            1100,
        ))
        .await
        .unwrap();

    let messages = store.load_messages("parent").await.unwrap();
    let store_arc: Arc<dyn Store> = store.clone();
    let chat = replay_into_chat("act", &messages, &store_arc, "parent").await;

    let sub = chat
        .blocks
        .iter()
        .find(|b| matches!(b, ChatBlock::Subagent { id, .. } if id == "task-run"))
        .expect("should have the subagent block");

    if let ChatBlock::Subagent {
        done, ok, summary, ..
    } = sub
    {
        assert!(*done, "interrupted subagent should display as done");
        assert!(!*ok, "interrupted subagent should display as failed");
        assert_eq!(summary, "(interrupted)");
    }
}
