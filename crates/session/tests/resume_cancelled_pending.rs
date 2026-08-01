//! `resume_and_replay` must leave `Cancelled` subagent tasks pending replay:
//! only `Running` children are replayed eagerly at resume; interrupted ones
//! stay `Cancelled` (no tool_result backfill, dangling `task` tool_use) until
//! the next user turn runs `replay_cancelled_tasks`.

use std::path::PathBuf;
use std::sync::Arc;

use opencoder_core::{ContentBlock, Message};
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_session::resume_and_replay;
use opencoder_store::{SubagentStatus, SubagentTaskRecord};

mod common;
use common::*;

#[tokio::test]
async fn resume_and_replay_leaves_cancelled_tasks_pending_replay() {
    // A Cancelled subagent (interrupted mid-run, no result) must NOT be
    // replayed by `resume_and_replay` — only Running children are replayed
    // there. Cancelled tasks stay Cancelled and keep their parent `task`
    // tool_use open so the next user turn replays them (`replay_cancelled_tasks`).
    let store = mem_store().await;
    store
        .create_session(&session_meta("parent", "act"))
        .await
        .unwrap();
    store
        .create_session(&session_meta("child-1", "explore"))
        .await
        .unwrap();
    store
        .append_message("parent", &Message::user("u1", "please explore"))
        .await
        .unwrap();
    store
        .append_message("parent", &parent_task_turn(&["task-cancelled"]))
        .await
        .unwrap();
    store
        .append_message("child-1", &Message::user("cu1", "explore the codebase"))
        .await
        .unwrap();
    store
        .create_subagent_task(&SubagentTaskRecord {
            task_id: "task-cancelled".into(),
            parent_session_id: "parent".into(),
            child_session_id: "child-1".into(),
            parent_message_id: Some("a1".into()),
            agent: "explore".into(),
            prompt: "explore the codebase".into(),
            result: None,
            status: SubagentStatus::Running,
            ok: None,
            started_at: 0,
            completed_at: None,
        })
        .await
        .unwrap();
    store.cancel_subagent_task("task-cancelled").await.unwrap();

    // No scripts queued — any LLM call would error. resume_and_replay must
    // make zero calls: it only replays Running children.
    let mock = Arc::new(MockChatClient::new());

    let session = resume_and_replay(
        store.clone(),
        "parent",
        config("m"),
        mock.clone() as Arc<dyn ChatStream>,
        PathBuf::from("/tmp"),
        None,
    )
    .await
    .unwrap();

    // (a) The task is still Cancelled (pending replay on the next user turn).
    let tasks = store.list_subagent_tasks("parent").await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert!(
        matches!(tasks[0].status, SubagentStatus::Cancelled),
        "Cancelled task must stay Cancelled after resume_and_replay, got {:?}",
        tasks[0].status
    );

    // (b) No tool_result was backfilled: the parent `task` tool_use remains
    //     open so `replay_cancelled_tasks` can run it on the next turn.
    let msgs = store.load_messages("parent").await.unwrap();
    let has_result = msgs.iter().any(|m| {
        m.blocks.iter().any(|b| {
            matches!(b,
                ContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == "task-cancelled")
        })
    });
    assert!(
        !has_result,
        "Cancelled task must not be backfilled during resume"
    );
    let dangling = dangling_tool_uses(&msgs);
    assert!(
        dangling.iter().any(|id| id == "task-cancelled"),
        "Cancelled task tool_use must stay dangling (replay pending), got {dangling:?}"
    );

    // (c) The resumed state also keeps the replayable tool_use dangling.
    let resumed_dangling = dangling_tool_uses(&session.messages);
    assert!(
        resumed_dangling.iter().any(|id| id == "task-cancelled"),
        "resumed SessionState must keep the Cancelled task tool_use open, got {resumed_dangling:?}"
    );

    // (d) Zero LLM calls: no Running child to continue, no Cancelled replay.
    assert_eq!(
        mock.call_count(),
        0,
        "resume_and_replay must not call the LLM for a Cancelled task"
    );
}
