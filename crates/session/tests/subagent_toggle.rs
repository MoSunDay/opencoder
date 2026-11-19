//! `subagent` config toggle (default ON) enforcement in the session runner.
//!
//! Contracts:
//! - `subagent: false` removes `task` from the advertised tool schemas, so
//!   the model is never offered a dispatch while the toggle is off.
//! - A hallucinated `task` call while disabled is denied with an error tool
//!   output, emits no SubagentStart/SubagentEnd, and persists no child
//!   session/task row.
//! - Default config keeps the toggle ON: `task` is advertised and a call
//!   still dispatches a child (regression guard for the default).
//! - Recovery replay (`replay_cancelled_tasks`) must not start children while
//!   the toggle is off; the interrupted task is backfilled with an error
//!   result and marked Failed instead of being re-run.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::{done_event, mem_store, parent_task_turn, session_meta};
use opencoder_core::{resolve_agent, Config, ContentBlock, Role};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::resume::replay_cancelled_tasks;
use opencoder_session::{run, SessionEvent, SessionState};
use opencoder_store::{SubagentStatus, SubagentTaskRecord};
use tokio_util::sync::CancellationToken;

fn default_config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

fn disabled_config() -> Config {
    Config {
        model: "m/g".into(),
        subagent: Some(false),
        ..Config::default()
    }
}

/// A parent turn that calls the `task` tool (with the toggle off this can
/// only be a hallucinated call — the schema is not advertised).
fn task_call_turn() -> LlmEvent {
    LlmEvent::Completed {
        text: "delegating".into(),
        tool_calls: vec![CompletedToolCall {
            id: "task-toggle-1".into(),
            name: "task".into(),
            input: serde_json::json!({"prompt": "research the codebase", "subagent_type": "explore"}),
        }],
        usage: Some(Usage {
            input_tokens: 10,
            output_tokens: 5,
            total_tokens: 15,
            ..Default::default()
        }),
    }
}

/// Tool names advertised in the FIRST LLM request of the run.
fn advertised_tool_names(mock: &MockChatClient) -> Vec<String> {
    let reqs = mock.requests();
    assert!(!reqs.is_empty(), "expected at least one LLM round");
    let mut names: Vec<String> = reqs[0]
        .tools
        .iter()
        .filter_map(|t| t["function"]["name"].as_str().map(str::to_string))
        .collect();
    names.sort();
    names
}

fn ev_name(e: &SessionEvent) -> &'static str {
    match e {
        SessionEvent::SubagentStart { .. } => "SubagentStart",
        SessionEvent::SubagentEnd { .. } => "SubagentEnd",
        SessionEvent::ToolEnd { .. } => "ToolEnd",
        _ => "Other",
    }
}

/// (a) With `subagent: false`, a model turn that calls `task` anyway gets an
/// error tool output, no subagent lifecycle events, and no child persisted.
#[tokio::test]
async fn disabled_toggle_denies_task_call_and_starts_no_child() {
    let store = mem_store().await;
    let sid = "sub-off-deny";
    store
        .create_session(&session_meta(sid, "act"))
        .await
        .unwrap();

    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![task_call_turn()]) // parent: (hallucinated) task call
            .push_script(vec![done_event("parent done")]), // parent: finish
    );
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut session = SessionState::new(
        sid,
        agent,
        disabled_config(),
        mock,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone());

    let mut events = Vec::new();
    run(&mut session, "delegate".into(), |ev| events.push(ev))
        .await
        .unwrap();

    // The task call is denied with the canonical disabled-toggle error.
    let denial = events.iter().find_map(|e| match e {
        SessionEvent::ToolEnd {
            name, output, is_error, ..
        } if name == "task" => Some((output.clone(), *is_error)),
        _ => None,
    });
    let (output, is_error) = denial.expect("task tool call must produce a ToolEnd");
    assert!(is_error, "denied task call must be an error output");
    assert!(
        output.contains("subagent dispatch is disabled"),
        "denial must name the toggle, got: {output}"
    );

    // No subagent lifecycle events (no child was started).
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SessionEvent::SubagentStart { .. })),
        "no SubagentStart while disabled, got: {:?}",
        events.iter().map(ev_name).collect::<Vec<_>>()
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SessionEvent::SubagentEnd { .. })),
        "no SubagentEnd while disabled, got: {:?}",
        events.iter().map(ev_name).collect::<Vec<_>>()
    );

    // No child session/task row was persisted.
    let tasks = store.list_subagent_tasks(sid).await.unwrap();
    assert!(
        tasks.is_empty(),
        "no subagent task row while disabled, got {} rows",
        tasks.len()
    );
}

/// (c) While disabled, the advertised tool schemas exclude `task`.
#[tokio::test]
async fn disabled_toggle_hides_task_from_advertised_tools() {
    let mock = Arc::new(MockChatClient::new().push_script(vec![done_event("ok")]));
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut session = SessionState::new(
        "sub-off-schema",
        agent,
        disabled_config(),
        mock.clone(),
        dir.path().to_path_buf(),
    );

    run(&mut session, "do the thing".into(), |_| {}).await.unwrap();

    let names = advertised_tool_names(&mock);
    assert!(
        !names.contains(&"task".to_string()),
        "task must not be advertised while disabled, got: {names:?}"
    );
    assert!(
        names.contains(&"bash".to_string()),
        "other tools stay advertised while disabled, got: {names:?}"
    );
}

/// (b)+(c) Default config: `task` stays advertised and still dispatches.
#[tokio::test]
async fn default_config_advertises_task_and_dispatches() {
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![task_call_turn()]) // parent: task call
            .push_script(vec![done_event("child done")]) // child round
            .push_script(vec![done_event("parent done")]), // parent: finish
    );
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut session = SessionState::new(
        "sub-on-default",
        agent,
        default_config(),
        mock.clone(),
        dir.path().to_path_buf(),
    );

    let mut events = Vec::new();
    run(&mut session, "delegate".into(), |ev| events.push(ev))
        .await
        .unwrap();

    let names = advertised_tool_names(&mock);
    assert!(
        names.contains(&"task".to_string()),
        "default config must advertise task, got: {names:?}"
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            SessionEvent::SubagentStart { kind, .. } if kind == "explore"
        )),
        "default config must still dispatch subagents, got: {:?}",
        events.iter().map(ev_name).collect::<Vec<_>>()
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::SubagentEnd { ok: true, .. })),
        "default-config subagent must complete ok, got: {:?}",
        events.iter().map(ev_name).collect::<Vec<_>>()
    );
}

/// (3) A disabled toggle must not start subagents during recovery replay:
/// the Cancelled child is never re-run, the dangling `task` tool_use is
/// backfilled with an error result, and the task is marked Failed.
#[tokio::test]
async fn replay_cancelled_tasks_skips_children_when_subagent_disabled() {
    let store = mem_store().await;
    store
        .create_session(&session_meta("parent-off", "act"))
        .await
        .unwrap();
    store
        .create_session(&session_meta("child-off", "explore"))
        .await
        .unwrap();
    store
        .append_message("parent-off", &parent_task_turn(&["task-off"]))
        .await
        .unwrap();
    store
        .append_message("child-off", &opencoder_core::Message::user("cu", "explore"))
        .await
        .unwrap();
    store
        .create_subagent_task(&SubagentTaskRecord {
            task_id: "task-off".into(),
            parent_session_id: "parent-off".into(),
            child_session_id: "child-off".into(),
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

    // Script that must NEVER be consumed while the toggle is off.
    let mock = Arc::new(MockChatClient::new().push_script(vec![done_event("must not run")]));
    let agent = resolve_agent("act").unwrap();
    let mut session = SessionState::new(
        "parent-off",
        agent,
        disabled_config(),
        mock.clone() as Arc<dyn ChatStream>,
        PathBuf::from("/tmp"),
    )
    .with_store(store.clone());
    // Make the dispatch tool_use visible so the task is replay-eligible —
    // the skip must come from the toggle, not the visibility guard.
    session.messages.push(parent_task_turn(&["task-off"]));
    session.cancel = Some(CancellationToken::new());

    replay_cancelled_tasks(&mut session, false).await;

    // The child was never started.
    assert_eq!(
        mock.call_count(),
        0,
        "disabled toggle must not start children during recovery"
    );

    // The task is terminal (Failed) with the disabled-toggle denial, so it
    // is never picked up by a later replay.
    let tasks = store.list_subagent_tasks("parent-off").await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert!(
        matches!(tasks[0].status, SubagentStatus::Failed),
        "skipped task must be marked Failed, got {:?}",
        tasks[0].status
    );
    assert!(
        tasks[0]
            .result
            .as_deref()
            .unwrap_or("")
            .contains("disabled"),
        "task result must carry the denial, got: {:?}",
        tasks[0].result
    );

    // The dangling tool_use got an error tool_result backfill.
    let backfilled = session.messages.iter().any(|m| {
        m.role == Role::Tool
            && m.blocks.iter().any(|b| matches!(
                b,
                ContentBlock::ToolResult { tool_use_id, is_error: true, .. }
                    if tool_use_id.as_str() == "task-off"
            ))
    });
    assert!(
        backfilled,
        "dangling task tool_use must be backfilled with an error result"
    );
}
