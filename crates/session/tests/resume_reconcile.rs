//! P2 functional tests for resume-time reconciliation:
//! - (1c) stuck Running subagents are marked Failed on resume
//! - (1d) compaction summary is persisted and applied on resume (head trim + summary prepend)

use std::path::PathBuf;
use std::sync::Arc;

use opencoder_core::{Config, ContentBlock, Message, Role};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::resume;
use opencoder_store::{
    LibsqlStore, SessionMeta, SessionPatch, Store, SubagentStatus, SubagentTaskRecord,
};

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
        }),
    }
}

fn client_done(text: &str) -> Arc<MockChatClient> {
    Arc::new(MockChatClient::new().with_default(vec![done_event(text)]))
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

/// (1c) Reconcile stuck Running subagents on resume.
///
/// When a process is interrupted mid-subagent, the task row is left in
/// `Running` state. On resume, these must be marked `Failed` with an
/// "(interrupted)" result so the transcript reflects reality and the task
/// isn't silently abandoned.
#[tokio::test]
async fn resume_marks_stuck_running_subagent_as_failed() {
    let store = mem_store().await;

    // Parent + child sessions (child needed for the subagent_tasks FK).
    store
        .create_session(&session_meta("parent", "act"))
        .await
        .unwrap();
    store
        .create_session(&session_meta("sub-child", "explore"))
        .await
        .unwrap();

    // A user message so resume has something to load.
    store
        .append_message("parent", &Message::user("u1", "hello"))
        .await
        .unwrap();

    // Subagent task stuck in Running (simulating an interrupted process).
    store
        .create_subagent_task(&SubagentTaskRecord {
            task_id: "task-stuck".into(),
            parent_session_id: "parent".into(),
            child_session_id: "sub-child".into(),
            parent_message_id: None,
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

    // Resume the parent session — this should reconcile the stuck task.
    let _resumed = resume(
        store.clone(),
        "parent",
        config("m"),
        client_done("x") as Arc<dyn ChatStream>,
        PathBuf::from("/tmp"),
    )
    .await
    .unwrap();

    // The stuck task must now be Failed with "(interrupted)" result.
    let tasks = store.list_subagent_tasks("parent").await.unwrap();
    assert_eq!(tasks.len(), 1, "expected the one subagent task");
    let t = &tasks[0];
    assert_eq!(t.task_id, "task-stuck");
    assert!(
        matches!(t.status, SubagentStatus::Failed),
        "stuck Running task must be marked Failed on resume, got {:?}",
        t.status
    );
    assert_eq!(t.ok, Some(false), "ok must be false");
    assert_eq!(
        t.result.as_deref(),
        Some("(interrupted)"),
        "result must be '(interrupted)', got: {:?}",
        t.result
    );
    assert!(t.completed_at.is_some(), "completed_at must be set");
}

/// (1c) Already-completed subagents are left untouched on resume.
#[tokio::test]
async fn resume_leaves_completed_subagent_untouched() {
    let store = mem_store().await;

    store
        .create_session(&session_meta("parent-done", "act"))
        .await
        .unwrap();
    store
        .create_session(&session_meta("sub-done", "explore"))
        .await
        .unwrap();
    store
        .append_message("parent-done", &Message::user("u1", "hi"))
        .await
        .unwrap();

    // A task that already completed successfully before the crash.
    store
        .create_subagent_task(&SubagentTaskRecord {
            task_id: "task-done".into(),
            parent_session_id: "parent-done".into(),
            child_session_id: "sub-done".into(),
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
    // Mark it completed (the normal path).
    store
        .complete_subagent_task("task-done", "found stuff", true)
        .await
        .unwrap();

    let _resumed = resume(
        store.clone(),
        "parent-done",
        config("m"),
        client_done("x") as Arc<dyn ChatStream>,
        PathBuf::from("/tmp"),
    )
    .await
    .unwrap();

    let tasks = store.list_subagent_tasks("parent-done").await.unwrap();
    assert_eq!(tasks.len(), 1);
    let t = &tasks[0];
    assert!(
        matches!(t.status, SubagentStatus::Completed),
        "already-completed task must stay Completed"
    );
    assert_eq!(t.ok, Some(true));
    assert_eq!(t.result.as_deref(), Some("found stuff"));
}

/// (1d) Compaction summary persistence + resume trimming.
///
/// When compaction has persisted `summary` + `summary_seq` to the session
/// meta, resume must:
///   1. Trim the summarized head (skip `summary_seq` messages)
///   2. Prepend a synthetic summary message
///   3. Carry the summary/summary_seq fields on the SessionState
#[tokio::test]
async fn resume_trims_summarized_head_and_prepends_summary() {
    let store = mem_store().await;

    store
        .create_session(&session_meta("compact-sess", "act"))
        .await
        .unwrap();

    // 5 messages: u1, a1, u2, a2, u3
    let msgs = vec![
        Message::user("u1", "first task"),
        Message::assistant("a1"),
        Message::user("u2", "second task"),
        Message::assistant("a2"),
        Message::user("u3", "third task"),
    ];
    store.append_messages("compact-sess", &msgs).await.unwrap();

    // Simulate a compaction that summarized the first 3 messages.
    store
        .update_session(
            "compact-sess",
            &SessionPatch {
                summary: Some("Summary of first 3 messages".into()),
                summary_seq: Some(3),
                updated_at: Some(opencoder_core::message::now_ms()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Resume — should trim the first 3 and prepend the summary.
    let resumed = resume(
        store.clone(),
        "compact-sess",
        config("m"),
        client_done("x") as Arc<dyn ChatStream>,
        PathBuf::from("/tmp"),
    )
    .await
    .unwrap();

    // 5 original - 3 skipped + 1 summary = 3 messages.
    assert_eq!(
        resumed.messages.len(),
        3,
        "expected 3 messages after trimming + summary prepend, got {}",
        resumed.messages.len()
    );

    // First message is the synthetic summary.
    assert!(
        resumed.messages[0].synthetic,
        "first message must be the synthetic compaction summary"
    );
    assert!(
        resumed.messages[0]
            .text()
            .starts_with("[Conversation summary so far]"),
        "first message must carry the compaction-summary prefix"
    );
    assert!(
        resumed.messages[0]
            .text()
            .contains("Summary of first 3 messages"),
        "first message must contain the persisted summary text, got: {}",
        resumed.messages[0].text()
    );

    // The remaining 2 messages are the tail: a2, u3 (original indices 3, 4).
    assert_eq!(
        resumed.messages[1].id, "a2",
        "second message must be the first tail message (a2)"
    );
    assert_eq!(
        resumed.messages[2].id, "u3",
        "third message must be the last tail message (u3)"
    );

    // The SessionState must carry the summary metadata.
    assert_eq!(
        resumed.summary.as_deref(),
        Some("Summary of first 3 messages"),
        "SessionState.summary must be restored from meta"
    );
    assert_eq!(
        resumed.summary_seq,
        Some(3),
        "SessionState.summary_seq must be restored from meta"
    );
}

/// (1d) Resume without compaction loads the full history unchanged.
#[tokio::test]
async fn resume_without_compaction_loads_full_history() {
    let store = mem_store().await;

    store
        .create_session(&session_meta("no-compact", "act"))
        .await
        .unwrap();

    let msgs = vec![
        Message::user("u1", "first"),
        Message::assistant("a1"),
        Message::user("u2", "second"),
    ];
    store.append_messages("no-compact", &msgs).await.unwrap();

    let resumed = resume(
        store.clone(),
        "no-compact",
        config("m"),
        client_done("x") as Arc<dyn ChatStream>,
        PathBuf::from("/tmp"),
    )
    .await
    .unwrap();

    // No compaction → all 3 messages loaded, no trimming, no summary.
    assert_eq!(resumed.messages.len(), 3);
    assert!(
        resumed.messages.iter().all(|m| !m.synthetic),
        "no synthetic summary message when no compaction occurred"
    );
    assert!(resumed.summary.is_none(), "summary must be None");
    assert!(resumed.summary_seq.is_none(), "summary_seq must be None");
}

fn assistant_with_tool_use(id: &str, tool_use_id: &str, name: &str) -> Message {
    let mut m = Message::assistant(id);
    m.blocks.push(ContentBlock::ToolUse {
        id: tool_use_id.into(),
        name: name.into(),
        input: serde_json::json!({}),
    });
    m
}

/// A hard-interrupt can leave an assistant tool_use with NO matching
/// tool_result. Resume must synthesize an error tool_result (persisted + in
/// memory) so the next LLM call doesn't get HTTP 400 for an unmatched id.
#[tokio::test]
async fn resume_synthesizes_error_result_for_dangling_tool_use() {
    let store = mem_store().await;
    store
        .create_session(&session_meta("dangling-sess", "act"))
        .await
        .unwrap();

    // user + assistant carrying an UNANSWERED tool_use.
    store
        .append_message("dangling-sess", &Message::user("u1", "do a thing"))
        .await
        .unwrap();
    store
        .append_message(
            "dangling-sess",
            &assistant_with_tool_use("a1", "call_dangling", "bash"),
        )
        .await
        .unwrap();

    let resumed = resume(
        store.clone(),
        "dangling-sess",
        config("m"),
        client_done("x") as Arc<dyn ChatStream>,
        PathBuf::from("/tmp"),
    )
    .await
    .unwrap();

    // 3 messages: user, assistant(tool_use), synthetic tool_result.
    assert_eq!(resumed.messages.len(), 3, "expected a synthetic tool result");
    let tool_msg = &resumed.messages[2];
    assert_eq!(tool_msg.role, Role::Tool, "synthesized msg must be Role::Tool");
    assert!(tool_msg.synthetic, "synthesized msg must be flagged synthetic");
    let result = tool_msg.blocks.iter().find_map(|b| match b {
        ContentBlock::ToolResult {
            tool_use_id,
            is_error,
            content,
            ..
        } => Some((tool_use_id.as_str(), *is_error, content.as_str())),
        _ => None,
    });
    let (id, is_error, _content) = result.expect("must contain a ToolResult for the dangling call");
    assert_eq!(id, "call_dangling", "must match the dangling tool_use id");
    assert!(is_error, "synthesized result must be an error");

    // Persisted too: a fresh load must include the synthetic message.
    let reloaded = store.load_messages("dangling-sess").await.unwrap();
    assert!(
        reloaded.iter().any(|m| {
            m.role == Role::Tool
                && m.blocks.iter().any(|b| {
                    matches!(
                        b,
                        ContentBlock::ToolResult {
                            tool_use_id,
                            is_error,
                            ..
                        } if tool_use_id == "call_dangling" && *is_error
                    )
                })
        }),
        "synthetic error result must be persisted to the store"
    );
}

/// When the assistant tool_use already has a matching tool_result, resume
/// must NOT inject a duplicate synthetic result.
#[tokio::test]
async fn resume_does_not_inject_when_tool_result_already_present() {
    let store = mem_store().await;
    store
        .create_session(&session_meta("paired-sess", "act"))
        .await
        .unwrap();

    // user + assistant(tool_use) + a real tool_result message (paired).
    store
        .append_message("paired-sess", &Message::user("u1", "do a thing"))
        .await
        .unwrap();
    store
        .append_message(
            "paired-sess",
            &assistant_with_tool_use("a1", "call_paired", "bash"),
        )
        .await
        .unwrap();
    let mut tool_msg = Message {
        id: "t1".into(),
        role: Role::Tool,
        blocks: vec![ContentBlock::ToolResult {
            tool_use_id: "call_paired".into(),
            content: "ran fine".into(),
            is_error: false,
        }],
        model: None,
        agent: None,
        usage: Default::default(),
        created_at: 0,
        synthetic: false,
    };
    let _ = &mut tool_msg; // keep field-order explicit above
    store.append_message("paired-sess", &tool_msg).await.unwrap();

    let resumed = resume(
        store.clone(),
        "paired-sess",
        config("m"),
        client_done("x") as Arc<dyn ChatStream>,
        PathBuf::from("/tmp"),
    )
    .await
    .unwrap();

    // Exactly the 3 original messages -- no synthetic injected.
    assert_eq!(
        resumed.messages.len(),
        3,
        "no synthetic message should be injected when the call is already answered"
    );
    assert!(
        !resumed.messages.iter().any(|m| {
            m.synthetic
                && m.blocks.iter().any(|b| {
                    matches!(
                        b,
                        ContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == "call_paired"
                    )
                })
        }),
        "must not inject a duplicate synthetic result"
    );

    // And the store row count is unchanged (no new row appended).
    let reloaded = store.load_messages("paired-sess").await.unwrap();
    assert_eq!(reloaded.len(), 3, "store must not gain a synthetic row");
}
