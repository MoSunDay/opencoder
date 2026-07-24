//! Resume/replay: when a parent session is hard-interrupted mid-subagent, the
//! task row is left `Running` and the parent transcript holds an unanswered
//! `task` tool_use. `resume_and_replay` resumes each stuck child, runs it to
//! completion, backfills the tool_result into the parent, and marks the task
//! complete. Children hold no `task` tool, so replay is exactly one level
//! (no recursion / no nested case).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use opencoder_core::{Config, ContentBlock, Message, MessageUsage, Role};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::resume_and_replay;
use opencoder_store::{LibsqlStore, SessionMeta, Store, SubagentStatus, SubagentTaskRecord};

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn config(model: &str) -> Config {
    Config {
        model: model.into(),
        ..Config::default()
    }
}

fn done_event(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.to_string(),
        tool_calls: Vec::<CompletedToolCall>::new(),
        usage: Some(Usage {
            input_tokens: 5,
            output_tokens: 3,
            total_tokens: 8,
            ..Default::default()
        }),
    }
}

fn session_meta(id: &str, agent: &str) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        title: Some("test".into()),
        agent: Some(agent.into()),
        model: Some("m".into()),
        workdir_hash: None,
        created_at: 0,
        updated_at: 0,
        summary: None,
        summary_seq: None,
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
    }
}

/// A parent assistant turn that emits one or more `task` tool_use blocks.
fn parent_task_turn(task_ids: &[&str]) -> Message {
    let mut blocks: Vec<ContentBlock> = vec![ContentBlock::Text {
        text: "delegating".into(),
    }];
    for id in task_ids {
        blocks.push(ContentBlock::ToolUse {
            id: (*id).into(),
            name: "task".into(),
            input: serde_json::json!({"prompt": "explore", "subagent_type": "explore"}),
        });
    }
    Message {
        id: "a1".into(),
        role: Role::Assistant,
        blocks,
        model: Some("m".into()),
        agent: Some("act".into()),
        usage: MessageUsage::default(),
        created_at: 0,
        synthetic: false,
    }
}

/// Collect the set of `tool_use` ids in `msgs` that have no matching
/// `tool_result` (i.e. would trigger dangling reconciliation).
fn dangling_tool_uses(msgs: &[Message]) -> Vec<String> {
    let answered: HashSet<&str> = msgs
        .iter()
        .flat_map(|m| m.blocks.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolResult { tool_use_id, .. } => Some(tool_use_id.as_str()),
            _ => None,
        })
        .collect();
    msgs.iter()
        .flat_map(|m| m.blocks.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolUse { id, .. } if !answered.contains(id.as_str()) => Some(id.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn resume_and_replay_continues_running_child_and_backfills_result() {
    let store = mem_store().await;
    store
        .create_session(&session_meta("parent", "act"))
        .await
        .unwrap();
    store
        .create_session(&session_meta("child-1", "explore"))
        .await
        .unwrap();
    // Parent transcript: user msg + assistant turn with a `task` tool_use.
    store
        .append_message("parent", &Message::user("u1", "please explore"))
        .await
        .unwrap();
    store
        .append_message("parent", &parent_task_turn(&["task-stuck"]))
        .await
        .unwrap();
    // Child transcript: the original dispatch prompt (history to continue from).
    store
        .append_message("child-1", &Message::user("cu1", "explore the codebase"))
        .await
        .unwrap();
    // The subagent task stuck in Running (interrupted mid-run).
    store
        .create_subagent_task(&SubagentTaskRecord {
            task_id: "task-stuck".into(),
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

    // Mock: the child's continuation produces a final answer.
    let mock =
        Arc::new(MockChatClient::new().push_script(vec![done_event("found 3 files: a, b, c")]));

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

    // (a) The task is now Completed with the child's result.
    let tasks = store.list_subagent_tasks("parent").await.unwrap();
    assert_eq!(tasks.len(), 1);
    let t = &tasks[0];
    assert_eq!(t.task_id, "task-stuck");
    assert!(
        matches!(t.status, SubagentStatus::Completed),
        "task must be Completed after replay, got {:?}",
        t.status
    );
    assert_eq!(t.ok, Some(true));
    assert!(
        t.result.as_deref().unwrap().contains("found 3 files"),
        "result must reflect child output: {:?}",
        t.result
    );

    // (b) Parent transcript backfilled a tool_result for task-stuck, and the
    //     task tool_use is no longer dangling (resume() did not synthesize an
    //     error result for it).
    let msgs = store.load_messages("parent").await.unwrap();
    let has_result = msgs.iter().any(|m| {
        m.blocks.iter().any(|b| {
            matches!(b,
                ContentBlock::ToolResult { tool_use_id, content, is_error }
                if tool_use_id == "task-stuck" && content.contains("found 3 files") && !is_error
            )
        })
    });
    assert!(has_result, "expected backfilled tool_result for task-stuck");
    let dangling = dangling_tool_uses(&msgs);
    assert!(
        dangling.is_empty(),
        "parent transcript must have no dangling tool_use after backfill: {:?}",
        dangling
    );

    // (c) The child transcript grew: the continuation assistant message landed.
    let child_msgs = store.load_messages("child-1").await.unwrap();
    assert!(
        child_msgs
            .iter()
            .any(|m| m.role == Role::Assistant && m.text().contains("found 3 files")),
        "child should have its continuation assistant message"
    );

    // (d) Exactly one LLM call — the child continuation (parent resume makes none).
    assert_eq!(
        mock.call_count(),
        1,
        "expected exactly 1 LLM call (child continuation)"
    );
}

#[tokio::test]
async fn resume_and_replay_leaves_completed_tasks_untouched() {
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
        .append_message("parent", &Message::user("u1", "hi"))
        .await
        .unwrap();
    // A task that already completed via the real lifecycle: create Running,
    // then complete it. `create_subagent_task` always inserts result=NULL
    // (only `complete_subagent_task` sets result/ok/completed_at).
    store
        .create_subagent_task(&SubagentTaskRecord {
            task_id: "task-done".into(),
            parent_session_id: "parent".into(),
            child_session_id: "child-1".into(),
            parent_message_id: None,
            agent: "explore".into(),
            prompt: "explore".into(),
            result: None,
            status: SubagentStatus::Running,
            ok: None,
            started_at: 0,
            completed_at: None,
        })
        .await
        .unwrap();
    store
        .complete_subagent_task("task-done", "already done", true)
        .await
        .unwrap();

    // No scripts queued — any LLM call would error.
    let mock = Arc::new(MockChatClient::new());

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

    // Completed tasks are never re-run.
    assert_eq!(mock.call_count(), 0, "completed task must not be re-run");
    let tasks = store.list_subagent_tasks("parent").await.unwrap();
    assert!(matches!(tasks[0].status, SubagentStatus::Completed));
    assert_eq!(tasks[0].result.as_deref(), Some("already done"));
}

#[tokio::test]
async fn resume_and_replay_no_running_tasks_just_resumes() {
    let store = mem_store().await;
    store
        .create_session(&session_meta("parent", "act"))
        .await
        .unwrap();
    store
        .append_message("parent", &Message::user("u1", "hello"))
        .await
        .unwrap();
    // No subagent tasks at all.

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

    assert_eq!(mock.call_count(), 0);
    assert_eq!(session.id, "parent");
    assert!(
        !session.messages.is_empty(),
        "messages should be reconstructed"
    );
}

#[tokio::test]
async fn resume_and_replay_replays_multiple_children_into_one_backfill_message() {
    let store = mem_store().await;
    store
        .create_session(&session_meta("parent", "act"))
        .await
        .unwrap();
    store
        .create_session(&session_meta("child-a", "explore"))
        .await
        .unwrap();
    store
        .create_session(&session_meta("child-b", "explore"))
        .await
        .unwrap();
    store
        .append_message("parent", &Message::user("u1", "delegate two"))
        .await
        .unwrap();
    // One parent assistant turn emitting TWO task tool_use blocks.
    store
        .append_message("parent", &parent_task_turn(&["task-A", "task-B"]))
        .await
        .unwrap();
    store
        .append_message("child-a", &Message::user("ca", "job A"))
        .await
        .unwrap();
    store
        .append_message("child-b", &Message::user("cb", "job B"))
        .await
        .unwrap();
    // Two Running tasks; A created first so it has the lower seq.
    for (tid, cid) in [("task-A", "child-a"), ("task-B", "child-b")] {
        store
            .create_subagent_task(&SubagentTaskRecord {
                task_id: tid.into(),
                parent_session_id: "parent".into(),
                child_session_id: cid.into(),
                parent_message_id: Some("a1".into()),
                agent: "explore".into(),
                prompt: "job".into(),
                result: None,
                status: SubagentStatus::Running,
                ok: None,
                started_at: 0,
                completed_at: None,
            })
            .await
            .unwrap();
    }

    // FIFO scripts: A replays first (lower seq), then B.
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

    // Both tasks Completed with their respective results.
    let tasks = store.list_subagent_tasks("parent").await.unwrap();
    assert_eq!(tasks.len(), 2);
    assert!(
        tasks
            .iter()
            .all(|t| matches!(t.status, SubagentStatus::Completed)),
        "both tasks must be Completed"
    );
    let by_id: HashMap<&str, &SubagentTaskRecord> =
        tasks.iter().map(|t| (t.task_id.as_str(), t)).collect();
    assert_eq!(by_id["task-A"].result.as_deref(), Some("result A"));
    assert_eq!(by_id["task-B"].result.as_deref(), Some("result B"));

    // Exactly ONE Tool message backfilled, holding both results in seq order.
    let msgs = store.load_messages("parent").await.unwrap();
    let tool_msgs: Vec<&Message> = msgs.iter().filter(|m| m.role == Role::Tool).collect();
    assert_eq!(
        tool_msgs.len(),
        1,
        "expected a single backfilled Tool message, got {}",
        tool_msgs.len()
    );
    let results: Vec<&str> = tool_msgs[0]
        .blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::ToolResult { tool_use_id, .. } => Some(tool_use_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        results,
        vec!["task-A", "task-B"],
        "results must be in dispatch (seq) order"
    );
    let dangling = dangling_tool_uses(&msgs);
    assert!(
        dangling.is_empty(),
        "no dangling tool_use after backfill: {dangling:?}"
    );

    assert_eq!(mock.call_count(), 2, "expected 2 child LLM calls");
}

// ---------------------------------------------------------------------------
// Regression: replay_cancelled_tasks respects the session cancel token.
// An interrupted (Cancelled) subagent must NOT be replayed when the parent
// cancel token is already fired -- the user double-Esc'd and expects no work.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn replay_cancelled_tasks_skips_children_when_cancel_token_fired() {
    use opencoder_session::SessionState;
    use tokio_util::sync::CancellationToken;

    let store = mem_store().await;
    store
        .create_session(&session_meta("parent", "act"))
        .await
        .unwrap();
    store
        .create_session(&session_meta("child-x", "explore"))
        .await
        .unwrap();
    store
        .append_message("parent", &Message::user("u1", "explore"))
        .await
        .unwrap();
    store
        .append_message("parent", &parent_task_turn(&["task-cx"]))
        .await
        .unwrap();
    store
        .append_message("child-x", &Message::user("cu", "explore"))
        .await
        .unwrap();
    store
        .create_subagent_task(&SubagentTaskRecord {
            task_id: "task-cx".into(),
            parent_session_id: "parent".into(),
            child_session_id: "child-x".into(),
            parent_message_id: None,
            agent: "explore".into(),
            prompt: "explore".into(),
            result: None,
            status: SubagentStatus::Cancelled,
            ok: None,
            started_at: 0,
            completed_at: None,
        })
        .await
        .unwrap();

    let mock = Arc::new(MockChatClient::new());
    let agent = opencoder_core::resolve_agent("act").unwrap();
    let mut session = SessionState::new(
        "parent",
        agent,
        config("m"),
        mock.clone() as Arc<dyn ChatStream>,
        PathBuf::from("/tmp"),
    )
    .with_store(store.clone());
    // Load the task tool_use into the session so the replay filter matches it.
    session.messages.push(parent_task_turn(&["task-cx"]));

    let token = CancellationToken::new();
    token.cancel();
    session.cancel = Some(token);

    opencoder_session::resume::replay_cancelled_tasks(&mut session).await;

    assert_eq!(
        mock.call_count(),
        0,
        "cancelled token must prevent child replay"
    );
    let tasks = store.list_subagent_tasks("parent").await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert!(
        matches!(tasks[0].status, SubagentStatus::Cancelled),
        "task must remain Cancelled, got {:?}",
        tasks[0].status
    );
}

#[tokio::test]
async fn replay_cancelled_tasks_runs_children_when_token_not_fired() {
    use opencoder_session::SessionState;
    use tokio_util::sync::CancellationToken;

    let store = mem_store().await;
    store
        .create_session(&session_meta("parent", "act"))
        .await
        .unwrap();
    store
        .create_session(&session_meta("child-y", "explore"))
        .await
        .unwrap();
    store
        .append_message("parent", &Message::user("u1", "explore"))
        .await
        .unwrap();
    store
        .append_message("parent", &parent_task_turn(&["task-cy"]))
        .await
        .unwrap();
    store
        .append_message("child-y", &Message::user("cu", "explore"))
        .await
        .unwrap();
    store
        .create_subagent_task(&SubagentTaskRecord {
            task_id: "task-cy".into(),
            parent_session_id: "parent".into(),
            child_session_id: "child-y".into(),
            parent_message_id: None,
            agent: "explore".into(),
            prompt: "explore".into(),
            result: None,
            status: SubagentStatus::Cancelled,
            ok: None,
            started_at: 0,
            completed_at: None,
        })
        .await
        .unwrap();

    let mock = Arc::new(MockChatClient::new().push_script(vec![done_event("explored: a, b")]));
    let agent = opencoder_core::resolve_agent("act").unwrap();
    let mut session = SessionState::new(
        "parent",
        agent,
        config("m"),
        mock.clone() as Arc<dyn ChatStream>,
        PathBuf::from("/tmp"),
    )
    .with_store(store.clone());
    // Load the task tool_use into the session so the replay filter matches it.
    session.messages.push(parent_task_turn(&["task-cy"]));

    session.cancel = Some(CancellationToken::new());

    opencoder_session::resume::replay_cancelled_tasks(&mut session).await;

    assert_eq!(
        mock.call_count(),
        1,
        "uncancelled token must allow child replay"
    );
    let tasks = store.list_subagent_tasks("parent").await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert!(
        matches!(tasks[0].status, SubagentStatus::Completed),
        "task must be Completed after replay, got {:?}",
        tasks[0].status
    );
    assert!(
        tasks[0].result.as_deref().unwrap().contains("explored"),
        "result must reflect child output: {:?}",
        tasks[0].result
    );
}

// ---------------------------------------------------------------------------
// Regression: a pending steer (user clicked steer-submit while a subagent was
// running) must cause cancelled subagents to be ABANDONED, not replayed.
// Otherwise the drain turn silently re-runs the child the user just
// interrupted, and the steer never gets processed (the steer is stuck).
// ---------------------------------------------------------------------------
#[tokio::test]
async fn replay_cancelled_tasks_abandons_when_steer_pending() {
    use opencoder_session::SessionState;
    use opencoder_store::{Delivery, SessionInput};
    use tokio_util::sync::CancellationToken;

    let store = mem_store().await;
    store
        .create_session(&session_meta("parent", "act"))
        .await
        .unwrap();
    store
        .create_session(&session_meta("child-z", "explore"))
        .await
        .unwrap();
    store
        .append_message("parent", &Message::user("u1", "explore"))
        .await
        .unwrap();
    store
        .append_message("parent", &parent_task_turn(&["task-cz"]))
        .await
        .unwrap();
    store
        .append_message("child-z", &Message::user("cu", "explore"))
        .await
        .unwrap();
    store
        .create_subagent_task(&SubagentTaskRecord {
            task_id: "task-cz".into(),
            parent_session_id: "parent".into(),
            child_session_id: "child-z".into(),
            parent_message_id: None,
            agent: "explore".into(),
            prompt: "explore".into(),
            result: None,
            status: SubagentStatus::Cancelled,
            ok: None,
            started_at: 0,
            completed_at: None,
        })
        .await
        .unwrap();

    // The user steered while the subagent was running: admit a pending steer.
    store
        .admit_input(&SessionInput {
            seq: None,
            id: "steer-1".into(),
            session_id: "parent".into(),
            delivery: Delivery::Steer,
            prompt: "forget that, do something else".into(),
            images: Vec::new(),
            admitted_seq: 0,
            promoted_seq: None,
        })
        .await
        .unwrap();

    let mock = Arc::new(MockChatClient::new().push_script(vec![done_event("should not run")]));
    let agent = opencoder_core::resolve_agent("act").unwrap();
    let mut session = SessionState::new(
        "parent",
        agent,
        config("m"),
        mock.clone() as Arc<dyn ChatStream>,
        PathBuf::from("/tmp"),
    )
    .with_store(store.clone());
    // Load the task tool_use into the session so the replay filter matches it.
    session.messages.push(parent_task_turn(&["task-cz"]));
    // Fresh (uncancelled) token — this is the drain turn after a steer-submit.
    session.cancel = Some(CancellationToken::new());

    opencoder_session::resume::replay_cancelled_tasks(&mut session).await;

    // The child must NOT be replayed.
    assert_eq!(
        mock.call_count(),
        0,
        "pending steer must prevent child replay"
    );
    // The task must be terminal (Failed), not Cancelled, so it is never
    // replayed again on a subsequent turn or resume.
    let tasks = store.list_subagent_tasks("parent").await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert!(
        matches!(tasks[0].status, SubagentStatus::Failed),
        "steered task must be Failed (abandoned), got {:?}",
        tasks[0].status
    );
    // A terminal tool_result must be backfilled so the transcript is well-formed.
    let dangling = dangling_tool_uses(&session.messages);
    assert!(
        dangling.is_empty(),
        "no dangling tool_use after abandon: {dangling:?}"
    );
    // The backfilled result must mention the steer/redirect.
    assert!(
        tasks[0].result.as_deref().unwrap().contains("steer"),
        "result must mention the steer redirect: {:?}",
        tasks[0].result
    );
}
