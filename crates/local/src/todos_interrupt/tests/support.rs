//! Fixtures for the `todos_interrupt` tests: a seeded workflow, the state a
//! live driver leaves while a batch is in flight, read-only access to the
//! durable state, the intent file and the interruption events, and a real
//! in-process driver.

use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use opencoder_core::Config;
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{Store, TursoStore};
use opencoder_todos::{
    domain, execution,
    interrupt_intent,
    persistence,
    types::{ContextMode, DispatchTodo, WorkflowStatus},
    WorkflowSpec, WorkflowState,
};
use tokio_util::sync::CancellationToken;

/// A live driver applies a request well within this bound.
pub(super) const LIVE_WAIT: Duration = Duration::from_secs(5);

pub(super) fn spec(id: &str) -> WorkflowSpec {
    opencoder_todos::parse_spec(&format!(
        r#"{{
            "schema_version": 1, "id": "{id}", "name": "interrupt request test",
            "objective": "verify the lock-outside interrupt contract", "constraints": [],
            "todos": [{{
                "id": "step-1", "title": "step",
                "requirement_background": "required by test",
                "instructions": "return a candidate", "depends_on": [],
                "agent": "act", "max_attempts": 1,
                "acceptance": {{"criteria": "candidate exists", "required_tool_calls": []}}
            }}],
            "metadata": null
        }}"#
    ))
    .unwrap()
}

pub(super) fn dispatch_decision() -> Vec<LlmEvent> {
    vec![LlmEvent::Completed {
        text: r#"{"operation":"dispatch","todos":[{"todo_id":"step-1","context_mode":"new"}],"reason":"ready"}"#
            .into(),
        tool_calls: Vec::new(),
        usage: None,
    }]
}

/// Persist workflow `id` in its initial (Pending) state on a writer that is
/// released again, so nothing in this process keeps the database open.
pub(super) async fn seed(workdir: &Path, workflow_id: &str) {
    let store = open_writer(workdir).await;
    let state = domain::initial_state(&spec(workflow_id), workflow_id.into(), "parent-1".into());
    opencoder_todos::parent::create_session(&store, &state, &Config::default())
        .await
        .unwrap();
    persistence::create(&store, &spec(workflow_id), &state)
        .await
        .unwrap();
}

/// A writer handle, opened exactly as `todos run` opens it.
pub(super) async fn open_writer(workdir: &Path) -> Arc<dyn Store> {
    Arc::new(crate::session_cmd::open_store(workdir).await.unwrap())
}

/// Move the persisted workflow to the state a live driver leaves it in
/// while a batch is in flight: dispatched (Running) with the TODO running.
pub(super) async fn move_to_running(store: &Arc<dyn Store>, workflow_id: &str) {
    let (spec, state) = persistence::load(store, workflow_id)
        .await
        .unwrap()
        .unwrap();
    execution::prepare_session(store, &spec, &spec.todos[0], "child-1", &Config::default())
        .await
        .unwrap();
    let running = opencoder_todos::transitions::dispatch(
        &spec,
        state,
        &[(
            DispatchTodo {
                todo_id: "step-1".into(),
                context_mode: ContextMode::New,
            },
            "child-1".into(),
        )],
    )
    .unwrap();
    assert_eq!(running.status, WorkflowStatus::Running);
    persistence::commit(store, &spec, &running, "todos_dispatched", serde_json::json!({}))
        .await
        .unwrap();
}

pub(super) async fn park(store: &Arc<dyn Store>, workflow_id: &str, reason: &str) {
    let (spec, state) = persistence::load(store, workflow_id)
        .await
        .unwrap()
        .unwrap();
    let suspended = opencoder_todos::transitions::terminal(
        state,
        WorkflowStatus::Suspended,
        reason.into(),
    )
    .unwrap();
    persistence::commit(
        store,
        &spec,
        &suspended,
        "workflow_interrupted",
        serde_json::json!({"reason": reason}),
    )
    .await
    .unwrap();
}

pub(super) async fn state(store: &Arc<dyn Store>, workflow_id: &str) -> WorkflowState {
    persistence::load(store, workflow_id)
        .await
        .unwrap()
        .expect("workflow must exist")
        .1
}

/// A read-only handle, opened as the inspecting commands open it.
pub(super) async fn reader(workdir: &Path) -> Arc<dyn Store> {
    Arc::new(
        TursoStore::open_reader(opencoder_core::data_dir_for(workdir).join("opencoder.db"))
            .await
            .unwrap(),
    )
}

pub(super) async fn pending_intent(workdir: &Path, workflow_id: &str) -> Option<interrupt_intent::InterruptIntent> {
    interrupt_intent::read(&interrupt_intent::root(workdir), workflow_id)
        .await
        .unwrap()
}

pub(super) async fn interrupt_events(store: &Arc<dyn Store>, workflow_id: &str) -> Vec<serde_json::Value> {
    store
        .todo_events_after(workflow_id, 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == "workflow_interrupted")
        .map(|event| event.payload)
        .collect()
}

pub(super) fn runtime(store: &Arc<dyn Store>, workdir: &Path, client: Arc<dyn ChatStream>) -> opencoder_todos::Runtime {
    opencoder_todos::Runtime {
        store: store.clone(),
        client,
        config: Config::default(),
        workdir: workdir.to_path_buf(),
        debug_root: None,
        cancel: CancellationToken::new(),
    }
}

/// Poll the durable state until `predicate` holds.
pub(super) async fn wait_until(store: &Arc<dyn Store>, workflow_id: &str, predicate: impl Fn(&WorkflowState) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let current = state(store, workflow_id).await;
        if predicate(&current) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "workflow never reached the awaited state: {:?}",
            current.status
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
