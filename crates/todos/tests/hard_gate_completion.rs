use std::sync::Arc;

use opencoder_core::Config;
use opencoder_llm::{CompletedToolCall, LlmEvent, MockChatClient};
use opencoder_store::{LibsqlStore, Store};
use opencoder_todos::{types::*, Runtime};
use tokio_util::sync::CancellationToken;

fn completed(text: &str, tool_calls: Vec<CompletedToolCall>) -> Vec<LlmEvent> {
    vec![LlmEvent::Completed {
        text: text.into(),
        tool_calls,
        usage: None,
    }]
}

fn candidate(text: &str) -> Vec<LlmEvent> {
    completed(text, Vec::new())
}

fn workflow() -> WorkflowSpec {
    WorkflowSpec {
        schema_version: 2,
        id: "hard-gate".into(),
        name: "hard gate".into(),
        objective: "execute once".into(),
        constraints: Vec::new(),
        todos: vec![TodoSpec {
            id: "step-1".into(),
            title: "one action".into(),
            requirement_background: "required".into(),
            instructions: "call bash once".into(),
            depends_on: Vec::new(),
            agent: "act".into(),
            max_attempts: 2,
            allowed_tools: vec!["bash".into()],
            acceptance: AcceptanceSpec {
                criteria: "tool succeeded".into(),
                required_tool_calls: vec![RequiredToolCall {
                    name: "bash".into(),
                    arguments_contains: serde_json::json!({"command":"printf done"}),
                    result_ok: true,
                }],
            },
            metadata: serde_json::Value::Null,
        }],
        metadata: serde_json::Value::Null,
    }
}

#[tokio::test]
async fn schema_v2_stops_after_last_required_tool_without_second_action_turn() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(candidate(
                r#"{"operation":"dispatch","todos":[{"todo_id":"step-1","context_mode":"new"}],"reason":"ready"}"#,
            ))
            .push_script(completed(
                "",
                vec![CompletedToolCall {
                    id: "call-1".into(),
                    name: "bash".into(),
                    input: serde_json::json!({"command":"printf done"}),
                }],
            ))
            .push_script(candidate(
                r#"{"operation":"accept","reason":"gate passed","mark_milestone":false}"#,
            ))
            .push_script(candidate(
                r#"{"operation":"complete","reason":"all passed"}"#,
            )),
    );
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime {
        store,
        client: mock.clone(),
        config: Config::default(),
        workdir: temp.path().to_path_buf(),
        debug_root: None,
        cancel: CancellationToken::new(),
    };

    let state = runtime
        .run_new_with_id(workflow(), "hard-gate-run".into())
        .await
        .unwrap();

    assert_eq!(state.status, WorkflowStatus::Completed);
    assert_eq!(state.todos["step-1"].status, TodoStatus::Passed);
    assert_eq!(state.todos["step-1"].attempt, 1);
    assert_eq!(mock.call_count(), 4, "the focused TODO used one model turn");
}
