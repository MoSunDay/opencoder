//! Proves that TODO env `env_vars` stamped by the dispatcher into
//! `metadata.env_vars` actually reach the tool processes of the child
//! session (`env_passthrough` -> `ToolContext::extra_env` -> bash `.envs()`),
//! and that a workflow without env_vars stays clean.

use std::sync::Arc;

use opencoder_core::Config;
use opencoder_core::ContentBlock;
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{LibsqlStore, Store};
use opencoder_todos::{types::*, Runtime};
use tokio_util::sync::CancellationToken;

fn done(text: &str) -> Vec<LlmEvent> {
    vec![LlmEvent::Completed {
        text: text.into(),
        tool_calls: Vec::new(),
        usage: None,
    }]
}

fn dispatch(todo_id: &str, context_mode: &str) -> Vec<LlmEvent> {
    done(&format!(
        r#"{{"operation":"dispatch","todos":[{{"todo_id":"{todo_id}","context_mode":"{context_mode}"}}],"reason":"ready"}}"#
    ))
}

const CANDIDATE: &str = r#"{"status":"candidate","summary":"done","result":"ok","verification":"checked","evidence_refs":["bash-output"],"recovery_context":{"summary":"done","refs":[]}}"#;

/// Exact command the probe round runs; the acceptance gate matches on the
/// full string (`json_contains` compares scalar values with equality).
const PROBE: &str = "echo probe-$OPENCODER_TODO_PROBE extra-$OPENCODER_TODO_EXTRA";

fn spec_with_env(id: &str, metadata: serde_json::Value) -> WorkflowSpec {
    let todo = TodoSpec {
        id: "step-1".into(),
        title: "step".into(),
        requirement_background: "required by test".into(),
        instructions: "echo the probe value".into(),
        depends_on: Vec::new(),
        agent: "act".into(),
        max_attempts: 3,
        acceptance: AcceptanceSpec {
            criteria: "bash probe succeeded".into(),
            required_tool_calls: vec![RequiredToolCall {
                name: "bash".into(),
                arguments_contains: serde_json::json!({"command": PROBE}),
                result_ok: true,
            }],
        },
        metadata: serde_json::Value::Null,
    };
    WorkflowSpec {
        schema_version: 1,
        id: id.into(),
        name: "env".into(),
        objective: "probe env".into(),
        constraints: Vec::new(),
        todos: vec![todo],
        metadata,
    }
}

fn bash_tool_turn(command: &str) -> Vec<LlmEvent> {
    vec![LlmEvent::Completed {
        text: "running the probe".into(),
        tool_calls: vec![opencoder_llm::CompletedToolCall {
            id: "t1".into(),
            name: "bash".into(),
            input: serde_json::json!({"command": command}),
        }],
        usage: None,
    }]
}

fn runtime(store: &Arc<dyn Store>, client: Arc<dyn ChatStream>, dir: &std::path::Path) -> Runtime {
    Runtime {
        store: store.clone(),
        client,
        config: Config::default(),
        workdir: dir.to_path_buf(),
        debug_root: None,
        cancel: CancellationToken::new(),
    }
}

async fn tool_result_texts(store: &Arc<dyn Store>, session_id: &str) -> Vec<String> {
    store
        .load_messages(session_id)
        .await
        .unwrap()
        .into_iter()
        .flat_map(|m| {
            m.blocks.into_iter().filter_map(|block| match block {
                ContentBlock::ToolResult { content, .. } => Some(content),
                _ => None,
            })
        })
        .collect()
}

#[tokio::test]
async fn stamped_env_vars_reach_bash_processes() {
    let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(dispatch("step-1", "new"))
            .push_script(bash_tool_turn(PROBE))
            .push_script(done(CANDIDATE))
            .push_script(done(
                r#"{"operation":"accept","reason":"meets criteria","mark_milestone":false}"#,
            ))
            .push_script(done(r#"{"operation":"complete","reason":"all passed"}"#)),
    );
    let temp = tempfile::tempdir().unwrap();
    let store_dyn: Arc<dyn Store> = store.clone();
    let runtime = runtime(&store_dyn, mock.clone(), temp.path());

    let spec = spec_with_env(
        "run-env",
        serde_json::json!({
            "env_vars": {
                "OPENCODER_TODO_EXTRA": "second-value",
                "OPENCODER_TODO_PROBE": "todo-env-value"
            }
        }),
    );
    let state = runtime
        .run_new_with_id(spec, "run-env".into())
        .await
        .unwrap();
    assert_eq!(state.status, WorkflowStatus::Completed);

    let state = opencoder_todos::persistence::load(&store_dyn, "run-env")
        .await
        .unwrap()
        .expect("workflow state exists")
        .1;
    let child_id = state.todos["step-1"].active_session_id.as_ref().unwrap();

    // The bash tool result must show the env values from the TODO env:
    // `env_passthrough_from_metadata` injected them into the child session's
    // `env_passthrough`, which `ToolContext::extra_env` forwards to bash.
    let results = tool_result_texts(&store_dyn, child_id).await;
    assert!(
        results
            .iter()
            .any(|content| content.contains("probe-todo-env-value")
                && content.contains("extra-second-value")),
        "bash output missing env values: {results:?}"
    );
    assert_eq!(mock.call_count(), 5);
}

#[tokio::test]
async fn workflows_without_env_vars_run_clean() {
    let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(dispatch("step-1", "new"))
            .push_script(done(CANDIDATE))
            .push_script(done(
                r#"{"operation":"accept","reason":"meets criteria","mark_milestone":false}"#,
            ))
            .push_script(done(r#"{"operation":"complete","reason":"all passed"}"#)),
    );
    let temp = tempfile::tempdir().unwrap();
    let store_dyn: Arc<dyn Store> = store.clone();
    let runtime = runtime(&store_dyn, mock.clone(), temp.path());

    let mut spec = spec_with_env("wf-clean", serde_json::Value::Null);
    spec.todos[0].acceptance.required_tool_calls = Vec::new();
    let state = runtime
        .run_new_with_id(spec, "run-clean".into())
        .await
        .unwrap();
    assert_eq!(state.status, WorkflowStatus::Completed);
    assert_eq!(mock.call_count(), 4);
}
