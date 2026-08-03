//! `resume_and_replay` must replay `Cancelled` subagent tasks (not just
//! `Running` ones) so they don't linger as "replay pending" in the TaskPicker.
//! Previously only `Running` children were replayed eagerly; `Cancelled`
//! tasks (from a prior hard-abort) stayed open until the next user turn.

use std::path::PathBuf;
use std::sync::Arc;

use opencoder_core::{ContentBlock, Message};
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_session::resume_and_replay;
use opencoder_store::{SubagentStatus, SubagentTaskRecord};

mod common;
use common::*;

#[tokio::test]
async fn resume_and_replay_replays_cancelled_task() {
    // A Cancelled subagent (interrupted mid-run) MUST be replayed by
    // `resume_and_replay` — same as a Running child. After replay the task
    // is Completed, a tool_result is backfilled, and the tool_use is no
    // longer dangling.
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
    // Simulate a prior hard-abort: the task is now Cancelled.
    store.cancel_subagent_task("task-cancelled").await.unwrap();

    // Mock: the child's continuation produces a final answer.
    let mock = Arc::new(
        MockChatClient::new().push_script(vec![done_event("found 3 files: a, b, c")]),
    );

    let _session = resume_and_replay(
        store.clone(),
        "parent",
        config("m"),
        mock.clone() as Arc<dyn ChatStream>,
        PathBuf::from("/tmp"),
        None,
    )
    .await
    .unwrap();

    // (a) The task is now Completed (no longer Cancelled).
    let tasks = store.list_subagent_tasks("parent").await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert!(
        matches!(tasks[0].status, SubagentStatus::Completed),
        "Cancelled task must be replayed to Completed, got {:?}",
        tasks[0].status
    );
    assert_eq!(tasks[0].ok, Some(true));
    assert!(
        tasks[0]
            .result
            .as_deref()
            .unwrap()
            .contains("found 3 files"),
        "result must reflect child output: {:?}",
        tasks[0].result
    );

    // (b) Parent transcript backfilled a tool_result; the tool_use is answered.
    let msgs = store.load_messages("parent").await.unwrap();
    let has_result = msgs.iter().any(|m| {
        m.blocks.iter().any(|b| {
            matches!(b,
                ContentBlock::ToolResult { tool_use_id, content, is_error, .. }
                if tool_use_id == "task-cancelled" && content.contains("found 3 files") && !is_error
            )
        })
    });
    assert!(
        has_result,
        "expected backfilled tool_result for task-cancelled"
    );
    let dangling = dangling_tool_uses(&msgs);
    assert!(
        dangling.is_empty(),
        "parent transcript must have no dangling tool_use after replay: {:?}",
        dangling
    );

    // (c) Exactly one LLM call — the child continuation.
    assert_eq!(
        mock.call_count(),
        1,
        "expected exactly 1 LLM call (cancelled child replay)"
    );
}

#[tokio::test]
async fn resume_and_replay_mixed_running_and_cancelled() {
    // Both Running and Cancelled tasks should be replayed in a single
    // resume_and_replay pass.
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
        .create_session(&session_meta("child-2", "explore"))
        .await
        .unwrap();
    store
        .append_message("parent", &Message::user("u1", "please explore"))
        .await
        .unwrap();
    store
        .append_message("parent", &parent_task_turn(&["task-running", "task-cancelled"]))
        .await
        .unwrap();
    store
        .append_message("child-1", &Message::user("cu1", "explore A"))
        .await
        .unwrap();
    store
        .append_message("child-2", &Message::user("cu2", "explore B"))
        .await
        .unwrap();

    for (tid, cid) in [("task-running", "child-1"), ("task-cancelled", "child-2")] {
        store
            .create_subagent_task(&SubagentTaskRecord {
                task_id: tid.into(),
                parent_session_id: "parent".into(),
                child_session_id: cid.into(),
                parent_message_id: Some("a1".into()),
                agent: "explore".into(),
                prompt: format!("explore {cid}"),
                result: None,
                status: SubagentStatus::Running,
                ok: None,
                started_at: 0,
                completed_at: None,
            })
            .await
            .unwrap();
    }
    // One Running, one Cancelled (prior hard-abort).
    store.cancel_subagent_task("task-cancelled").await.unwrap();

    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_event("result A")])
            .push_script(vec![done_event("result B")]),
    );

    let _session = resume_and_replay(
        store.clone(),
        "parent",
        config("m"),
        mock.clone() as Arc<dyn ChatStream>,
        PathBuf::from("/tmp"),
        None,
    )
    .await
    .unwrap();

    let tasks = store.list_subagent_tasks("parent").await.unwrap();
    assert_eq!(tasks.len(), 2);
    for t in &tasks {
        assert!(
            matches!(t.status, SubagentStatus::Completed),
            "task {} must be Completed, got {:?}",
            t.task_id,
            t.status
        );
    }
    assert_eq!(mock.call_count(), 2, "both children must be replayed");
}
