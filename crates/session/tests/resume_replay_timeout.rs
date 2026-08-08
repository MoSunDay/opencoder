//! Replay-timeout & cancel tests for `resume_and_replay` / `replay_child`.
//!
//! Contracts:
//! - replay_child respects `config.replay_timeout()`: a wedged child (LLM stream
//!   that never produces an event) is cut off instead of hanging recovery forever.
//! - replay_child is abortable via `replay_cancel` (parent cancellation).
//! - in both cases the interrupted task is marked `Failed` and an error
//!   `tool_result` is backfilled into the parent so the transcript stays
//!   well-formed and the user turn can proceed.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use opencoder_core::{Config, ContentBlock, Message, MessageUsage, Role};
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent};
use opencoder_session::resume_and_replay;
use opencoder_store::{LibsqlStore, SessionMeta, Store, SubagentStatus, SubagentTaskRecord};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// A `ChatStream` whose stream never produces an event and never closes — the
/// child's LLM call awaits the first event forever, exercising the replay
/// timeout / cancel paths rather than erroring immediately.
struct HangingChatClient;

impl ChatStream for HangingChatClient {
    fn chat_stream(&self, _req: ChatRequest) -> Result<mpsc::Receiver<LlmEvent>> {
        let (tx, rx) = mpsc::channel::<LlmEvent>(8);
        // Hold the sender forever so `rx.recv()` awaits indefinitely.
        tokio::spawn(async move {
            std::future::pending::<()>().await;
            drop(tx);
        });
        Ok(rx)
    }
    fn backend(&self) -> &'static str {
        "hanging-mock"
    }
}

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
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
        summary_images: vec![],
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
        requirement: None,
    }
}

fn parent_task_turn(task_id: &str) -> Message {
    Message {
        id: "a1".into(),
        role: Role::Assistant,
        blocks: vec![
            ContentBlock::Text {
                text: "delegating".into(),
            },
            ContentBlock::ToolUse {
                id: task_id.into(),
                name: "task".into(),
                input: serde_json::json!({"prompt": "explore", "subagent_type": "explore"}),
            },
        ],
        model: Some("m".into()),
        agent: Some("act".into()),
        usage: MessageUsage::default(),
        created_at: 0,
        synthetic: false,
    }
}

/// Seed a parent + child session with a `Running` subagent task whose child
/// transcript holds the dispatch prompt (so the child resumes and re-issues an
/// LLM call that hangs on the `HangingChatClient`).
async fn seed_running_subagent(store: &Arc<dyn Store>) {
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
        .append_message("parent", &parent_task_turn("task-stuck"))
        .await
        .unwrap();
    store
        .append_message("child-1", &Message::user("cu1", "explore the codebase"))
        .await
        .unwrap();
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
}

#[tokio::test]
async fn replay_child_times_out_and_marks_task_failed() {
    let store = mem_store().await;
    seed_running_subagent(&store).await;

    // 1s replay timeout — the child hangs, so recovery must cut it off at 1s
    // rather than waiting for the 600s stream idle timeout.
    let config = Config {
        model: "m".into(),
        replay_timeout_secs: Some(1),
        ..Config::default()
    };
    let client = Arc::new(HangingChatClient) as Arc<dyn ChatStream>;

    let start = std::time::Instant::now();
    let session = resume_and_replay(
        store.clone(),
        "parent",
        config,
        client,
        PathBuf::from("/tmp"),
        None,
    )
    .await
    .expect("resume must return (not hang)");
    let elapsed = start.elapsed();

    // (a) Did not hang: returned well under the 600s idle timeout.
    assert!(
        elapsed.as_secs() < 30,
        "recovery must not hang: took {:?}",
        elapsed
    );

    // (b) The wedged task is now Failed (ok=false), not left Running.
    let tasks = store.list_subagent_tasks("parent").await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert!(
        matches!(tasks[0].status, SubagentStatus::Failed),
        "wedged task must be Failed after replay timeout, got {:?}",
        tasks[0].status
    );
    assert_eq!(tasks[0].ok, Some(false));

    // (c) Parent transcript has an error tool_result so the turn can proceed.
    let msgs = store.load_messages("parent").await.unwrap();
    let has_error_result = msgs.iter().any(|m| {
        m.blocks.iter().any(|b| {
            matches!(b,
                ContentBlock::ToolResult { tool_use_id, is_error, .. }
                if tool_use_id == "task-stuck" && *is_error
            )
        })
    });
    assert!(
        has_error_result,
        "expected an error tool_result backfilled for the timed-out task"
    );

    // (d) The session was reconstructed (resume ran after the replay).
    assert_eq!(session.id, "parent");
    assert!(!session.messages.is_empty());
}

#[tokio::test]
async fn replay_child_aborts_on_parent_cancel() {
    let store = mem_store().await;
    seed_running_subagent(&store).await;

    // Long replay timeout so the timeout does NOT fire; only the cancel token
    // aborts the child.
    let config = Config {
        model: "m".into(),
        replay_timeout_secs: Some(300),
        ..Config::default()
    };
    let client = Arc::new(HangingChatClient) as Arc<dyn ChatStream>;
    let cancel = CancellationToken::new();

    // Cancel almost immediately so the child is cut off before the 300s timeout.
    let cancel_clone = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        cancel_clone.cancel();
    });

    let start = std::time::Instant::now();
    let session = resume_and_replay(
        store.clone(),
        "parent",
        config,
        client,
        PathBuf::from("/tmp"),
        Some(cancel),
    )
    .await
    .expect("resume must return (not hang)");
    let elapsed = start.elapsed();

    // Cancelled within ~1s, nowhere near the 300s timeout.
    assert!(
        elapsed.as_secs() < 30,
        "recovery must abort promptly on parent cancel: took {:?}",
        elapsed
    );

    // Task marked Failed.
    let tasks = store.list_subagent_tasks("parent").await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert!(
        matches!(tasks[0].status, SubagentStatus::Failed),
        "cancelled-replay task must be Failed, got {:?}",
        tasks[0].status
    );
    assert_eq!(session.id, "parent");
}
