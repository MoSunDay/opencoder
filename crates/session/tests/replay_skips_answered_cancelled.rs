//! Regression: a Cancelled subagent task whose `tool_use` already has a matching
//! `tool_result` (e.g. a timed-out subagent) must NOT be replayed or abandoned.
//! Doing so would append a duplicate tool_result that providers reject with
//! HTTP 400.

use std::path::PathBuf;
use std::sync::Arc;

use opencoder_core::{Config, ContentBlock, Message, MessageUsage, Role};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::SessionState;
use opencoder_store::{LibsqlStore, SessionMeta, Store, SubagentStatus, SubagentTaskRecord};
use tokio_util::sync::CancellationToken;

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
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
        summary_images: vec![],
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
        requirement: None,
    }
}

/// Count how many tool_result blocks exist for a given tool_use_id.
fn count_tool_results(msgs: &[Message], tool_use_id: &str) -> usize {
    msgs.iter()
        .flat_map(|m| m.blocks.iter())
        .filter(|b| {
            matches!(
                b,
                ContentBlock::ToolResult { tool_use_id: id, .. } if id.as_str() == tool_use_id
            )
        })
        .count()
}

#[tokio::test]
async fn replay_skips_cancelled_task_with_existing_tool_result() {
    let store = mem_store().await;
    store
        .create_session(&session_meta("parent-to", "act"))
        .await
        .unwrap();
    store
        .create_session(&session_meta("child-to", "explore"))
        .await
        .unwrap();

    // Parent transcript: user msg + assistant task tool_use + tool_result (timeout).
    store
        .append_message("parent-to", &Message::user("u1", "explore"))
        .await
        .unwrap();

    // Assistant turn with a task tool_use.
    let assistant_msg = Message {
        id: "a1".into(),
        role: Role::Assistant,
        blocks: vec![
            ContentBlock::Text {
                text: "delegating".into(),
            },
            ContentBlock::ToolUse {
                id: "task-to".into(),
                name: "task".into(),
                input: serde_json::json!({"prompt": "explore", "subagent_type": "explore"}),
            },
        ],
        model: Some("m".into()),
        agent: Some("act".into()),
        usage: MessageUsage::default(),
        created_at: 0,
        synthetic: false,
    };
    store
        .append_message("parent-to", &assistant_msg)
        .await
        .unwrap();

    // The timeout already recorded a tool_result for this task_use.
    let tool_msg = Message {
        id: "t1".into(),
        role: Role::Tool,
        blocks: vec![ContentBlock::ToolResult {
            tool_use_id: "task-to".into(),
            content: "subagent timed out after 1800s without completing".into(),
            is_error: true,
            images: Vec::new(),
        }],
        model: None,
        agent: None,
        usage: MessageUsage::default(),
        created_at: 0,
        synthetic: false,
    };
    store.append_message("parent-to", &tool_msg).await.unwrap();

    store
        .append_message("child-to", &Message::user("cu", "explore"))
        .await
        .unwrap();

    // The task is Cancelled (as the timeout fix sets it).
    store
        .create_subagent_task(&SubagentTaskRecord {
            task_id: "task-to".into(),
            parent_session_id: "parent-to".into(),
            child_session_id: "child-to".into(),
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

    // Mock with a script that must NEVER be consumed.
    let mock = Arc::new(MockChatClient::new().push_script(vec![done_event("should not run")]));
    let agent = opencoder_core::resolve_agent("act").unwrap();
    let mut session = SessionState::new(
        "parent-to",
        agent,
        Config {
            model: "m".into(),
            ..Config::default()
        },
        mock.clone() as Arc<dyn ChatStream>,
        PathBuf::from("/tmp"),
    )
    .with_store(store.clone());
    session.messages.push(assistant_msg);
    session.messages.push(tool_msg);
    session.cancel = Some(CancellationToken::new());

    // Call replay_cancelled_tasks (no new input, no pending steers/queues → would
    // normally try to replay the Cancelled child).
    opencoder_session::resume::replay_cancelled_tasks(&mut session, false).await;

    // The child must NOT have been replayed.
    assert_eq!(
        mock.call_count(),
        0,
        "cancelled task with existing tool_result must not be replayed"
    );

    // Task status must remain Cancelled (unchanged).
    let tasks = store.list_subagent_tasks("parent-to").await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert!(
        matches!(tasks[0].status, SubagentStatus::Cancelled),
        "task must remain Cancelled (not replayed or abandoned), got {:?}",
        tasks[0].status
    );

    // No duplicate tool_result.
    let result_count = count_tool_results(&session.messages, "task-to");
    assert_eq!(
        result_count, 1,
        "exactly one tool_result expected (no duplicate), got {result_count}"
    );
}
