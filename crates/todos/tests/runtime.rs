use std::sync::Arc;

use opencoder_core::Config;
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{LibsqlStore, Store, TASK_TYPE_TODO, TASK_TYPE_TODO_WORKFLOW};
use opencoder_todos::{types::*, Runtime};
use tokio_util::sync::CancellationToken;

fn done(text: &str) -> Vec<LlmEvent> {
    vec![LlmEvent::Completed {
        text: text.into(),
        tool_calls: Vec::new(),
        usage: None,
    }]
}

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
            acceptance: AcceptanceSpec {
                criteria: "candidate exists".into(),
                required_tool_calls: Vec::new(),
            },
            metadata: serde_json::Value::Null,
        }],
        metadata: serde_json::Value::Null,
    }
}

#[tokio::test]
async fn parent_drives_focused_primary_todo_to_completion() {
    let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(done(r#"{"operation":"dispatch","todos":[{"todo_id":"step-1","context_mode":"new"}],"reason":"ready"}"#))
            .push_script(done(r#"{"status":"candidate","summary":"done","result":"ok","verification":"checked","evidence_refs":[],"recovery_context":{"summary":"done","refs":[]}}"#))
            .push_script(done(r#"{"operation":"accept","reason":"meets criteria","mark_milestone":true}"#))
            .push_script(done(r#"{"operation":"complete","reason":"all passed"}"#)),
    );
    let client: Arc<dyn ChatStream> = mock.clone();
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime {
        store: store.clone(),
        client,
        config: Config::default(),
        workdir: temp.path().to_path_buf(),
        debug_root: Some(temp.path().join("dump")),
        cancel: CancellationToken::new(),
    };
    let state = runtime
        .run_new_with_id(spec(), "run-test".into())
        .await
        .unwrap();
    assert_eq!(state.status, WorkflowStatus::Completed);
    assert_eq!(state.todos["step-1"].status, TodoStatus::Passed);
    assert!(state.milestones.contains("step-1"));
    assert!(temp
        .path()
        .join("dump/run-test/task-info/index.json")
        .is_file());

    let parent = store
        .get_session(&state.parent_session_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(parent.task_type.as_deref(), Some(TASK_TYPE_TODO_WORKFLOW));
    let child_id = state.todos["step-1"].active_session_id.as_ref().unwrap();
    let child = store.get_session(child_id).await.unwrap().unwrap();
    assert_eq!(child.task_type.as_deref(), Some(TASK_TYPE_TODO));
    assert_eq!(mock.call_count(), 4);
}

#[tokio::test]
async fn normal_execution_does_not_create_a_debug_projection() {
    let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(done(r#"{"operation":"dispatch","todos":[{"todo_id":"step-1","context_mode":"new"}],"reason":"ready"}"#))
            .push_script(done(r#"{"status":"candidate","summary":"done","result":"ok","verification":"checked","evidence_refs":[],"recovery_context":{"summary":"done","refs":[]}}"#))
            .push_script(done(r#"{"operation":"accept","reason":"meets criteria","mark_milestone":false}"#))
            .push_script(done(r#"{"operation":"complete","reason":"all passed"}"#)),
    );
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime {
        store,
        client: mock,
        config: Config::default(),
        workdir: temp.path().to_path_buf(),
        debug_root: None,
        cancel: CancellationToken::new(),
    };

    let state = runtime
        .run_new_with_id(spec(), "run-no-debug".into())
        .await
        .unwrap();

    assert_eq!(state.status, WorkflowStatus::Completed);
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn parent_can_dispatch_multiple_independent_todos_in_one_batch() {
    let mut workflow = spec();
    let mut second = workflow.todos[0].clone();
    second.id = "step-2".into();
    second.title = "second".into();
    workflow.todos.push(second);
    let candidate = r#"{"status":"candidate","summary":"done","result":"ok","verification":"checked","evidence_refs":[],"recovery_context":{"summary":"done","refs":[]}}"#;
    let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(done(r#"{"operation":"dispatch","todos":[{"todo_id":"step-1","context_mode":"new"},{"todo_id":"step-2","context_mode":"new"}],"reason":"independent"}"#))
            .push_script(done(candidate))
            .push_script(done(candidate))
            .push_script(done(r#"{"operation":"accept","reason":"ok","mark_milestone":false}"#))
            .push_script(done(r#"{"operation":"accept","reason":"ok","mark_milestone":false}"#))
            .push_script(done(r#"{"operation":"complete","reason":"all passed"}"#)),
    );
    let client: Arc<dyn ChatStream> = mock.clone();
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime {
        store,
        client,
        config: Config::default(),
        workdir: temp.path().to_path_buf(),
        debug_root: None,
        cancel: CancellationToken::new(),
    };

    let state = runtime
        .run_new_with_id(workflow, "run-parallel".into())
        .await
        .unwrap();

    assert_eq!(state.status, WorkflowStatus::Completed);
    assert_eq!(state.todos["step-1"].status, TodoStatus::Passed);
    assert_eq!(state.todos["step-2"].status, TodoStatus::Passed);
    assert_eq!(mock.call_count(), 6);
}

#[test]
fn dependency_validation_rejects_cycles_and_runnable_is_dependency_aware() {
    let mut workflow = spec();
    workflow.todos.push(TodoSpec {
        id: "step-2".into(),
        title: "second".into(),
        requirement_background: "second".into(),
        instructions: "second".into(),
        depends_on: vec!["step-1".into()],
        agent: "act".into(),
        max_attempts: 1,
        acceptance: AcceptanceSpec {
            criteria: "second".into(),
            required_tool_calls: vec![],
        },
        metadata: serde_json::Value::Null,
    });
    opencoder_todos::domain::validate_spec(&workflow).unwrap();
    let state = opencoder_todos::domain::initial_state(&workflow, "run".into(), "parent".into());
    assert_eq!(
        opencoder_todos::domain::runnable(&workflow, &state),
        vec!["step-1"]
    );
    workflow.todos[0].depends_on = vec!["step-2".into()];
    assert!(opencoder_todos::domain::validate_spec(&workflow).is_err());
}

#[test]
fn suspended_active_todo_becomes_recoverable_and_runnable() {
    let workflow = spec();
    let state =
        opencoder_todos::domain::initial_state(&workflow, "run-recovery".into(), "parent".into());
    let state = opencoder_todos::transitions::dispatch(
        &workflow,
        state,
        &[(
            DispatchTodo {
                todo_id: "step-1".into(),
                context_mode: ContextMode::New,
            },
            "child".into(),
        )],
    )
    .unwrap();
    let state = opencoder_todos::transitions::terminal(
        state,
        WorkflowStatus::Suspended,
        "app crashed".into(),
    )
    .unwrap();

    assert!(state.active_todo_ids.is_empty());
    assert_eq!(state.todos["step-1"].status, TodoStatus::Interrupted);
    assert_eq!(
        opencoder_todos::domain::runnable(&workflow, &state),
        vec!["step-1"]
    );
}
