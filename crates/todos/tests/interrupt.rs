//! Interrupt and concurrency-conflict recovery for the TODO runtime: a held
//! in-flight LLM call (MockChatClient::push_hang) is interrupted either by an
//! external store-level interrupt, a local cancellation, or an external
//! generation bump — the run must stop, persist a non-running workflow, and
//! stay resumable.

use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use opencoder_core::Config;
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{LibsqlStore, Store};
use opencoder_todos::{types::*, Runtime};
use tokio_util::sync::CancellationToken;

const CANDIDATE: &str = r#"{"status":"candidate","summary":"done","result":"ok","verification":"checked","evidence_refs":[],"recovery_context":{"summary":"done","refs":[]}}"#;

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

fn runtime(store: &Arc<dyn Store>, client: Arc<dyn ChatStream>, dir: &Path) -> Runtime {
    Runtime {
        store: store.clone(),
        client,
        config: Config::default(),
        workdir: dir.to_path_buf(),
        debug_root: None,
        cancel: CancellationToken::new(),
    }
}

async fn load(store: &Arc<dyn Store>, id: &str) -> WorkflowState {
    opencoder_todos::persistence::load(store, id)
        .await
        .unwrap()
        .expect("workflow must be persisted")
        .1
}

async fn kinds(store: &Arc<dyn Store>, id: &str) -> Vec<String> {
    store
        .todo_events_after(id, 0)
        .await
        .unwrap()
        .into_iter()
        .map(|event| event.kind)
        .collect()
}

/// Poll the durable item projections until the TODO shows as running (the
/// batch committed "todos_dispatched" and execution is in flight).
async fn wait_until_running(store: &Arc<dyn Store>, workflow_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let items = store.list_todo_items(workflow_id).await.unwrap();
        if items.iter().any(|item| item.status == "running") {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "todo in {workflow_id} never reached running"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn external_interrupt_cancels_inflight_todo_and_is_resumable() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let hang = Arc::new(tokio::sync::Notify::new());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(dispatch("step-1", "new"))
            .push_hang(hang.clone()),
    );
    let temp = tempfile::tempdir().unwrap();
    let run_runtime = Arc::new(runtime(&store, mock.clone(), temp.path()));
    let spawned = {
        let rt = run_runtime.clone();
        tokio::spawn(async move { rt.run_new_with_id(spec(), "run-interrupt".into()).await })
    };

    wait_until_running(&store, "run-interrupt").await;
    opencoder_todos::interrupt(&store, "run-interrupt", "test interrupt")
        .await
        .unwrap();
    hang.notify_one();

    // The run must stop promptly — Ok or a persisted Err, never hung or
    // silently continuing.
    let outcome = tokio::time::timeout(Duration::from_secs(10), spawned)
        .await
        .expect("run task finished after interrupt")
        .unwrap();
    let state = load(&store, "run-interrupt").await;
    assert_eq!(state.status, WorkflowStatus::Suspended);
    assert_ne!(
        state.todos["step-1"].status,
        TodoStatus::Running,
        "in-flight TODO must be torn down"
    );
    assert!(
        state.todos["step-1"].active_session_id.is_some(),
        "child session kept for resume"
    );
    let events = kinds(&store, "run-interrupt").await;
    assert!(events.contains(&"workflow_interrupted".into()));
    if let Ok(finished) = &outcome {
        assert_eq!(finished.status, WorkflowStatus::Suspended);
    }

    // A fresh process (fresh mock, same store) resumes to completion.
    let mock2 = Arc::new(
        MockChatClient::new()
            .push_script(dispatch("step-1", "resume"))
            .push_script(done(CANDIDATE))
            .push_script(done(
                r#"{"operation":"accept","reason":"ok","mark_milestone":false}"#,
            ))
            .push_script(done(r#"{"operation":"complete","reason":"all passed"}"#)),
    );
    let resumed = runtime(&store, mock2, temp.path())
        .resume("run-interrupt")
        .await
        .unwrap();

    assert_eq!(resumed.status, WorkflowStatus::Completed);
    assert_eq!(resumed.todos["step-1"].status, TodoStatus::Passed);
    assert_eq!(resumed.todos["step-1"].attempt, 2);
}

#[tokio::test]
async fn local_cancel_mid_todo_marks_item_interrupted_and_stops_cleanly() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let hang = Arc::new(tokio::sync::Notify::new());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(dispatch("step-1", "new"))
            .push_hang(hang.clone()),
    );
    let temp = tempfile::tempdir().unwrap();
    let cancel = CancellationToken::new();
    let mut run_config = runtime(&store, mock, temp.path());
    run_config.cancel = cancel.clone();
    let run_runtime = Arc::new(run_config);
    let spawned = {
        let rt = run_runtime.clone();
        tokio::spawn(async move { rt.run_new_with_id(spec(), "run-cancel".into()).await })
    };

    wait_until_running(&store, "run-cancel").await;
    cancel.cancel();
    hang.notify_one();

    let state = tokio::time::timeout(Duration::from_secs(10), spawned)
        .await
        .expect("run task finished after local cancel")
        .unwrap()
        .expect("local cancel resolves to Ok(suspended state)");

    assert_eq!(state.status, WorkflowStatus::Suspended);
    assert_eq!(state.todos["step-1"].status, TodoStatus::Interrupted);
    let events = store.todo_events_after("run-cancel", 0).await.unwrap();
    let failed = events
        .iter()
        .find(|event| event.kind == "todo_execution_failed")
        .expect("per-item failure is recorded");
    assert_eq!(failed.payload["todo_id"], "step-1");
    assert_eq!(
        failed.payload["interrupted"], true,
        "cancellation maps the item to Interrupted, not Failed"
    );
    assert!(events
        .iter()
        .any(|event| event.kind == "workflow_interrupted"));
}

#[tokio::test]
async fn generation_conflict_stops_the_run() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let hang = Arc::new(tokio::sync::Notify::new());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(dispatch("step-1", "new"))
            .push_hang(hang.clone()),
    );
    let temp = tempfile::tempdir().unwrap();
    let run_runtime = Arc::new(runtime(&store, mock.clone(), temp.path()));
    let spawned = {
        let rt = run_runtime.clone();
        tokio::spawn(async move { rt.run_new_with_id(spec(), "run-conflict".into()).await })
    };

    wait_until_running(&store, "run-conflict").await;
    // An external writer bumps the generation behind the runtime's back.
    let (workflow, mut state) = opencoder_todos::persistence::load(&store, "run-conflict")
        .await
        .unwrap()
        .unwrap();
    state.generation += 1;
    opencoder_todos::persistence::commit(
        &store,
        &workflow,
        &state,
        "external_change",
        serde_json::json!({}),
    )
    .await
    .unwrap();
    hang.notify_one();

    // The run must observe the conflict and stop instead of steamrolling it.
    let outcome = tokio::time::timeout(Duration::from_secs(10), spawned)
        .await
        .expect("run task finished after generation conflict")
        .unwrap();
    let final_state = load(&store, "run-conflict").await;
    assert_ne!(
        final_state.status,
        WorkflowStatus::Running,
        "workflow must not be left running unattended"
    );
    let events = kinds(&store, "run-conflict").await;
    assert!(events.contains(&"external_change".into()));
    if outcome.is_ok() {
        assert!(
            events.iter().any(|kind| matches!(
                kind.as_str(),
                "workflow_suspended" | "runtime_error" | "workflow_interrupted"
            )),
            "an Ok outcome must have persisted a suspension"
        );
    }
    assert_eq!(
        mock.call_count(),
        2,
        "no further LLM scheduling after the conflict"
    );
}

#[tokio::test]
async fn interrupt_rejects_terminal_workflow() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(dispatch("step-1", "new"))
            .push_script(done(CANDIDATE))
            .push_script(done(
                r#"{"operation":"accept","reason":"ok","mark_milestone":false}"#,
            ))
            .push_script(done(r#"{"operation":"complete","reason":"all passed"}"#)),
    );
    let temp = tempfile::tempdir().unwrap();
    let runtime = runtime(&store, mock, temp.path());

    let state = runtime
        .run_new_with_id(spec(), "run-done".into())
        .await
        .unwrap();
    assert_eq!(state.status, WorkflowStatus::Completed);

    let error = opencoder_todos::interrupt(&store, "run-done", "too late")
        .await
        .unwrap_err();

    assert!(format!("{error:#}").contains("cannot interrupt terminal workflow"));
}

/// Bug #5 regression: with max_attempts=1, an external store-level interrupt
/// that lands while the local cancel token has not been flipped yet (the
/// `poll_interrupt` 250ms window) must NOT be mistaken for a plain execution
/// failure — the TODO must stay Interrupted (not Failed) and the externally
/// persisted Suspended state must survive untouched.
#[tokio::test]
async fn external_interrupt_window_keeps_max_attempt_todo_unfailed() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let hang = Arc::new(tokio::sync::Notify::new());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(dispatch("step-1", "new"))
            .push_hang(hang.clone()),
    );
    let temp = tempfile::tempdir().unwrap();
    // A runtime whose local cancel token is never touched: only the external
    // store write interrupts the run, exercising the polling window.
    let run_runtime = Arc::new(runtime(&store, mock, temp.path()));
    let spawned = {
        let rt = run_runtime.clone();
        tokio::spawn(async move {
            let mut workflow = spec();
            workflow.todos[0].max_attempts = 1;
            rt.run_new_with_id(workflow, "run-window".into()).await
        })
    };

    wait_until_running(&store, "run-window").await;
    opencoder_todos::interrupt(&store, "run-window", "external stop")
        .await
        .unwrap();
    // Release the in-flight call immediately — the child session fails while
    // the local token is still unset, the exact misclassification window.
    hang.notify_one();

    let outcome = tokio::time::timeout(Duration::from_secs(10), spawned)
        .await
        .expect("run task finished after external interrupt window")
        .unwrap();
    if let Ok(finished) = &outcome {
        assert_eq!(finished.status, WorkflowStatus::Suspended);
    }

    // The externally written Suspended verdict must be intact: not Failed,
    // not re-committed from the stale local Running state.
    let state = load(&store, "run-window").await;
    assert_eq!(state.status, WorkflowStatus::Suspended);
    assert_eq!(state.terminal_reason.as_deref(), Some("external stop"));
    assert_ne!(
        state.todos["step-1"].status,
        TodoStatus::Failed,
        "a max_attempts=1 TODO must not fail from an external interrupt"
    );
    assert_eq!(
        state.todos["step-1"].status,
        TodoStatus::Interrupted,
        "the external suspension verdict for the in-flight TODO must survive"
    );
    assert_eq!(state.todos["step-1"].attempt, 1);
    assert!(
        state.todos["step-1"].active_session_id.is_some(),
        "child session kept for resume"
    );

    // The local execution-failure bookkeeping must never have been applied.
    let events = kinds(&store, "run-window").await;
    assert!(
        !events.iter().any(|kind| kind == "todo_execution_failed"),
        "the external interrupt must not be recorded as a local execution failure"
    );
    assert!(events.contains(&"workflow_interrupted".into()));
}

/// Bug #5 regression (runner half): when a local cancel races an external
/// store write, `drive_inner`'s interrupt branch must adopt the externally
/// persisted state instead of committing a local "workflow_interrupted"
/// (with "local interrupt requested") over it.
#[tokio::test]
async fn local_cancel_after_external_write_adopts_external_state() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let hang = Arc::new(tokio::sync::Notify::new());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(dispatch("step-1", "new"))
            .push_hang(hang.clone()),
    );
    let temp = tempfile::tempdir().unwrap();
    let cancel = CancellationToken::new();
    let mut run_config = runtime(&store, mock, temp.path());
    run_config.cancel = cancel.clone();
    let run_runtime = Arc::new(run_config);
    let spawned = {
        let rt = run_runtime.clone();
        tokio::spawn(async move { rt.run_new_with_id(spec(), "run-mixed".into()).await })
    };

    wait_until_running(&store, "run-mixed").await;
    // External verdict first, then a local cancel while the batch is still
    // holding the in-flight call.
    opencoder_todos::interrupt(&store, "run-mixed", "external stop")
        .await
        .unwrap();
    cancel.cancel();
    hang.notify_one();

    let outcome = tokio::time::timeout(Duration::from_secs(10), spawned)
        .await
        .expect("run task finished after mixed interrupt")
        .unwrap()
        .expect("mixed interrupt resolves to the adopted external state");

    assert_eq!(outcome.status, WorkflowStatus::Suspended);
    assert_eq!(outcome.terminal_reason.as_deref(), Some("external stop"));

    // The store keeps the external verdict; the runtime's local interrupt
    // commit must not have overwritten it.
    let state = load(&store, "run-mixed").await;
    assert_eq!(state.status, WorkflowStatus::Suspended);
    assert_eq!(state.terminal_reason.as_deref(), Some("external stop"));
    assert_eq!(state.todos["step-1"].status, TodoStatus::Interrupted);
    assert_ne!(state.todos["step-1"].status, TodoStatus::Failed);
    assert_eq!(state.todos["step-1"].attempt, 1);

    let events = kinds(&store, "run-mixed").await;
    assert!(
        !events.iter().any(|kind| kind == "todo_execution_failed"),
        "no local execution-failure bookkeeping over the external interrupt"
    );
    let interrupts: Vec<_> = store
        .todo_events_after("run-mixed", 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == "workflow_interrupted")
        .collect();
    assert_eq!(
        interrupts.len(),
        1,
        "exactly the external workflow_interrupted commit must exist"
    );
    assert_eq!(interrupts[0].payload["reason"], "external stop");
}
