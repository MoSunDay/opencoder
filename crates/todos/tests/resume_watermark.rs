//! Bug #6 regression: a Resume-mode TODO session carries the previous
//! attempt's transcript, including its assistant Candidate JSON. When the
//! current run produces no new assistant message, execution must fail with
//! "no final candidate" instead of recycling the stale message.

use std::sync::Arc;

use opencoder_core::{message::now_ms, Config, Message};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{LibsqlStore, SessionMeta, Store, TASK_TYPE_TODO};
use opencoder_todos::{execution, types::*};
use tokio_util::sync::CancellationToken;

const STALE_CANDIDATE: &str = r#"{"status":"candidate","summary":"stale","result":"old","verification":"stale","evidence_refs":[],"recovery_context":{"summary":"stale","refs":[]}}"#;

fn spec() -> WorkflowSpec {
    WorkflowSpec {
        schema_version: 1,
        id: "wf-test".into(),
        name: "test".into(),
        objective: "finish one item".into(),
        constraints: Vec::new(),
        todos: vec![TodoSpec {
            id: "step-1".into(),
            title: "step".into(),
            requirement_background: "required by test".into(),
            instructions: "return the candidate".into(),
            depends_on: Vec::new(),
            agent: "act".into(),
            max_attempts: 2,
            allowed_tools: vec![],
            acceptance: AcceptanceSpec {
                criteria: "candidate exists".into(),
                required_tool_calls: Vec::new(),
            },
            metadata: serde_json::Value::Null,
        }],
        metadata: serde_json::Value::Null,
    }
}

fn assistant_text(id: &str, text: &str) -> Message {
    let mut message = Message::assistant(id);
    message.blocks = vec![opencoder_core::ContentBlock::text(text)];
    message
}

/// Seed a durable session whose transcript already ends with an assistant
/// message containing a perfectly valid (but stale) Candidate JSON.
async fn seed_session_with_stale_assistant(store: &Arc<dyn Store>, config: &Config, id: &str) {
    let now = now_ms();
    store
        .create_session(&SessionMeta {
            id: id.into(),
            title: Some("test / step".into()),
            agent: Some("act".into()),
            model: Some(config.model.clone()),

            autopilot_mode: None,
            workdir_hash: None,
            created_at: now,
            updated_at: now,
            summary: None,
            summary_seq: None,
            summary_images: Vec::new(),
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: Some(TASK_TYPE_TODO.into()),
            requirement: Some("return the candidate".into()),
            plan_snapshot: None,
            plan_input_count: 0,
        })
        .await
        .unwrap();
    store
        .import_messages(
            id,
            &[
                Message::user("u1", "previous attempt prompt"),
                assistant_text("a1", STALE_CANDIDATE),
            ],
        )
        .await
        .unwrap();
}

/// A resume whose run adds no assistant message (the session's cancel token
/// is already fired, so `run` stops at the first turn boundary) must NOT
/// return the stale transcript candidate.
#[tokio::test]
async fn resume_without_new_assistant_rejects_stale_candidate() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(MockChatClient::new());
    let client: Arc<dyn ChatStream> = mock.clone();
    let config = Config::default();
    seed_session_with_stale_assistant(&store, &config, "todo-sess").await;

    let workflow = spec();
    let mut state =
        opencoder_todos::domain::initial_state(&workflow, "wf-run".into(), "parent".into());
    {
        let todo = state.todos.get_mut("step-1").unwrap();
        todo.status = TodoStatus::Running;
        todo.attempt = 2;
        todo.active_session_id = Some("todo-sess".into());
        todo.session_history = vec!["todo-sess".into()];
    }

    let cancel = CancellationToken::new();
    cancel.cancel();
    let temp = tempfile::tempdir().unwrap();
    let result = execution::execute(
        store.clone(),
        client,
        config,
        temp.path(),
        &workflow,
        &state,
        &workflow.todos[0],
        ContextMode::Resume,
        "todo-sess".into(),
        cancel,
    )
    .await;

    let error = match result {
        Err(error) => format!("{error:#}"),
        Ok(execution) => panic!(
            "stale candidate was recycled instead of rejected: {}",
            execution.candidate.summary
        ),
    };
    assert!(
        error.contains("no final candidate"),
        "expected the stale assistant message to be rejected, got: {error}"
    );
    // The run never reached the LLM: the run loop stopped at its first turn
    // boundary, so no assistant message was appended.
    assert_eq!(mock.call_count(), 0);
}

/// Positive control: when the resumed run DOES append an assistant message,
/// exactly that message (not the stale pre-watermark one) becomes the
/// candidate.
#[tokio::test]
async fn resume_with_new_assistant_uses_only_the_new_candidate() {
    let fresh_candidate = STALE_CANDIDATE.replace("stale", "fresh");
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(MockChatClient::new().push_script(vec![LlmEvent::Completed {
        text: fresh_candidate.clone(),
        tool_calls: Vec::new(),
        usage: None,
    }]));
    let client: Arc<dyn ChatStream> = mock.clone();
    let config = Config::default();
    seed_session_with_stale_assistant(&store, &config, "todo-sess").await;

    let workflow = spec();
    let mut state =
        opencoder_todos::domain::initial_state(&workflow, "wf-run".into(), "parent".into());
    {
        let todo = state.todos.get_mut("step-1").unwrap();
        todo.status = TodoStatus::Running;
        todo.attempt = 2;
        todo.active_session_id = Some("todo-sess".into());
        todo.session_history = vec!["todo-sess".into()];
    }

    let temp = tempfile::tempdir().unwrap();
    let execution = execution::execute(
        store.clone(),
        client,
        config,
        temp.path(),
        &workflow,
        &state,
        &workflow.todos[0],
        ContextMode::Resume,
        "todo-sess".into(),
        CancellationToken::new(),
    )
    .await
    .unwrap();

    assert_eq!(execution.candidate.summary, "fresh");
    assert_eq!(mock.call_count(), 1);
}
