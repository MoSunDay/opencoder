//! Subagent × interleaved thinking — cross-path regression tests.
//!
//! Subagents inherit `interleaved_thinking` and `reasoning_effort` from the
//! parent's `Config` (cloned wholesale in `run_subagent`). The parent-only
//! tests in `interleaved_thinking.rs` and the reasoning-free subagent tests in
//! `subagent.rs` never exercise the *intersection*: a child whose tool-call
//! turn emits `ReasoningDelta`. These tests close that gap.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, ContentBlock, Role};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionState};
use opencoder_store::{LibsqlStore, Store};

// ---------------------------------------------------------------------------
// event builders
// ---------------------------------------------------------------------------

fn reasoning_delta(text: &str) -> LlmEvent {
    LlmEvent::ReasoningDelta(text.to_string())
}

fn completed_with_tools(text: &str, tool_calls: Vec<CompletedToolCall>) -> LlmEvent {
    LlmEvent::Completed {
        text: text.to_string(),
        tool_calls,
        usage: Some(Usage {
            input_tokens: 10,
            output_tokens: 20,
            total_tokens: 30,
        }),
    }
}

fn completed_no_tools(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.to_string(),
        tool_calls: Vec::new(),
        usage: Some(Usage {
            input_tokens: 10,
            output_tokens: 20,
            total_tokens: 30,
        }),
    }
}

fn bash_tool_call() -> CompletedToolCall {
    CompletedToolCall {
        id: "child-call-1".into(),
        name: "bash".into(),
        input: serde_json::json!({ "command": "echo hello" }),
    }
}

/// Parent turn that dispatches an `explore` subagent via the `task` tool.
fn task_turn(prompt: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: "delegating".into(),
        tool_calls: vec![CompletedToolCall {
            id: "task-1".into(),
            name: "task".into(),
            input: serde_json::json!({"prompt": prompt, "subagent_type": "explore"}),
        }],
        usage: Some(Usage {
            input_tokens: 10,
            output_tokens: 5,
            total_tokens: 15,
        }),
    }
}

// ---------------------------------------------------------------------------
// session helpers
// ---------------------------------------------------------------------------

fn config_with_interleave(on: bool) -> Config {
    Config {
        model: "main/glm-5.2".into(),
        interleaved_thinking: Some(on),
        ..Config::default()
    }
}

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

/// Shared mock script for a subagent that does ONE tool-call turn with
/// reasoning before finishing:
///
///   script[0] parent turn 1 -> task tool call (dispatch child)
///   script[1] child  turn 1 -> ReasoningDelta + bash tool call
///   script[2] child  turn 2 -> plain text done
///   script[3] parent turn 2 -> plain text done
fn reasoning_subagent_mock() -> Arc<MockChatClient> {
    Arc::new(
        MockChatClient::new()
            .push_script(vec![task_turn("explore the repo")])
            .push_script(vec![
                reasoning_delta("Let me analyze the repo structure..."),
                completed_with_tools("", vec![bash_tool_call()]),
            ])
            .push_script(vec![completed_no_tools("child finished")])
            .push_script(vec![completed_no_tools("parent done")]),
    )
}

/// Create a parent session row (FK prerequisite for subagent_tasks).
async fn seed_parent(store: &Arc<dyn Store>, id: &str) {
    store
        .create_session(&opencoder_store::SessionMeta {
            id: id.into(),
            title: Some("test".into()),
            agent: Some("act".into()),
            model: Some("main/glm-5.2".into()),
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
        })
        .await
        .unwrap();
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

/// A child whose tool-call turn emits reasoning must persist a `Reasoning`
/// block into its durable message transcript (proving `interleaved_thinking`
/// inherited from the parent config takes effect in the subagent code path).
#[tokio::test]
async fn subagent_reasoning_persisted_on_child_tool_call_turn() {
    let store = mem_store().await;
    seed_parent(&store, "parent-rit-1").await;

    let mock = reasoning_subagent_mock();
    let client: Arc<dyn ChatStream> = mock.clone();
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut session = SessionState::new(
        "parent-rit-1",
        agent,
        config_with_interleave(true),
        client,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone());

    run(&mut session, "delegate".into(), |_| {})
        .await
        .unwrap();

    // Locate the child session via the parent-child task record.
    let tasks = store.list_subagent_tasks("parent-rit-1").await.unwrap();
    assert_eq!(tasks.len(), 1, "expected one subagent task");
    let child_id = tasks[0].child_session_id.clone();

    // The child's first assistant turn (tool-call turn) must contain a
    // Reasoning block.
    let child_msgs = store.load_messages(&child_id).await.unwrap();
    let first_asst = child_msgs
        .iter()
        .find(|m| m.role == Role::Assistant)
        .expect("child must have an assistant message");
    let has_reasoning = first_asst
        .blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Reasoning { .. }));
    assert!(
        has_reasoning,
        "child tool-call turn must persist a Reasoning block, got blocks: {:?}",
        first_asst.blocks
    );
}

/// The child's second `chat_stream` request must re-serialize
/// `reasoning_content` in the assistant message, proving the persisted
/// Reasoning block round-trips back to the model via `push_assistant`.
#[tokio::test]
async fn subagent_reasoning_sent_back_in_child_second_request() {
    let mock = reasoning_subagent_mock();
    let client: Arc<dyn ChatStream> = mock.clone();
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut session = SessionState::new(
        "parent-rit-2",
        agent,
        config_with_interleave(true),
        client,
        dir.path().to_path_buf(),
    );

    run(&mut session, "delegate".into(), |_| {})
        .await
        .unwrap();

    // Four chat_stream calls in deterministic order:
    //   [0] parent turn 1   [1] child turn 1   [2] child turn 2   [3] parent turn 2
    let reqs = mock.requests();
    assert!(
        reqs.len() >= 4,
        "expected >=4 requests (2 parent + 2 child), got {}",
        reqs.len()
    );

    // The child's second request (index 2) must include reasoning_content
    // in an assistant message.
    let child_second = &reqs[2];
    let has_reasoning = child_second
        .messages
        .iter()
        .filter(|m| m.get("role").and_then(|v| v.as_str()) == Some("assistant"))
        .any(|m| m.get("reasoning_content").is_some());
    assert!(
        has_reasoning,
        "child's second request must include reasoning_content in an assistant message"
    );
}

/// With `interleaved_thinking = false`, the child must NOT persist Reasoning
/// blocks even on tool-call turns.
#[tokio::test]
async fn subagent_interleaved_disabled_skips_child_persistence() {
    let store = mem_store().await;
    seed_parent(&store, "parent-rit-3").await;

    let mock = reasoning_subagent_mock();
    let client: Arc<dyn ChatStream> = mock.clone();
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut session = SessionState::new(
        "parent-rit-3",
        agent,
        config_with_interleave(false),
        client,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone());

    run(&mut session, "delegate".into(), |_| {})
        .await
        .unwrap();

    let tasks = store.list_subagent_tasks("parent-rit-3").await.unwrap();
    let child_id = tasks[0].child_session_id.clone();
    let child_msgs = store.load_messages(&child_id).await.unwrap();

    let any_reasoning = child_msgs
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .flat_map(|m| &m.blocks)
        .any(|b| matches!(b, ContentBlock::Reasoning { .. }));
    assert!(
        !any_reasoning,
        "interleaved_thinking=false must NOT persist reasoning in child messages"
    );
}

/// The child's first `ChatRequest` must carry the parent's
/// `reasoning_effort`, proving the Config was inherited wholesale.
#[tokio::test]
async fn subagent_inherits_reasoning_effort() {
    let mut config = config_with_interleave(true);
    config.reasoning_effort = Some("high".into());

    let mock = reasoning_subagent_mock();
    let client: Arc<dyn ChatStream> = mock.clone();
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut session = SessionState::new(
        "parent-rit-4",
        agent,
        config,
        client,
        dir.path().to_path_buf(),
    );

    run(&mut session, "delegate".into(), |_| {})
        .await
        .unwrap();

    // The child's first request (index 1) must carry reasoning_effort=high.
    let reqs = mock.requests();
    assert!(reqs.len() >= 2, "need >=2 requests");
    assert_eq!(
        reqs[1].reasoning_effort.as_deref(),
        Some("high"),
        "child must inherit reasoning_effort from parent config"
    );
}
