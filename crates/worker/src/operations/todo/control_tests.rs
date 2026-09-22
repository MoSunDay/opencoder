use super::*;
use crate::{
    lifecycle::{Lifecycle, TodoInitialization},
    WorkerOptions,
};
use opencoder_llm::MockChatClient;
use opencoder_node::fleet::NodeService;
use opencoder_store::SessionMeta;
use std::sync::Arc;

async fn fixture_with_count(
    count: usize,
) -> (
    tempfile::TempDir,
    Worker,
    WorkflowSpec,
    WorkflowState,
    RerunRequest,
) {
    let root = tempfile::tempdir().unwrap();
    let workdir = root.path().join("work");
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    std::fs::write(workdir.join(".opencoder/ap.json"), r#"{"mode":"off"}"#).unwrap();
    let worker = Worker::open(
        WorkerOptions {
            name: "recovery".into(),
            workdir,
            data_dir: root.path().join("data"),
            workflow_root: None,
            max_runs: Some(1),
            dag: false,
        },
        Some(Arc::new(MockChatClient::new())),
    )
    .await
    .unwrap();
    let mut spec:WorkflowSpec=serde_json::from_value(json!({"schema_version":1,"id":"definition","name":"test","objective":"recover","todos":[{"id":"a","title":"A","requirement_background":"test","instructions":"execute","acceptance":{"criteria":"passed"}}]})).unwrap();
    if count > 1 {
        spec.todos = (0..count)
            .map(|index| {
                let mut todo = spec.todos[0].clone();
                todo.id = format!("item-{index}");
                todo
            })
            .collect();
    }
    let mut state = opencoder_todos::domain::initial_state(
        &spec,
        "todos-recover".into(),
        "parent-recover".into(),
    );
    state.status = WorkflowStatus::Completed;
    for todo in state.todos.values_mut() {
        todo.status = TodoStatus::Passed;
    }
    worker
        .inner
        .state
        .store
        .create_session(&SessionMeta {
            id: state.parent_session_id.clone(),
            created_at: 1,
            updated_at: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    persistence::create(&worker.inner.state.store, &spec, &state)
        .await
        .unwrap();
    let request = RerunRequest {
        request_id: "rerun-recover".into(),
        todo_id: spec.todos[0].id.clone(),
        reason: "recover durable command".into(),
        expected_generation: state.generation,
    };
    let node = worker.registration().id;
    let record = Record {
        annotations: Value::Null,
        queue: None,
        assignment: Assignment {
            private_context: None,
            runtime: None,
            codex: None,
            index: ExecutionIndex {
                id: state.workflow_id.clone(),
                kind: ExecutionKind::Todos,
                node_id: node.clone(),
                created_at: 1,
                status: ExecutionStatus::Done,
            },
            request: CreateExecution {
                id: state.workflow_id.clone(),
                kind: ExecutionKind::Todos,
                target: None,
                node_id: Some(node),
                input: json!({}),
            },
            definition: Some(json!(spec)),
        },
        result: Value::Null,
        error: None,
        events: vec![],
        lifecycle: Lifecycle {
            todo_initialization: Some(TodoInitialization::Initialized),
            ..Default::default()
        },
    };
    worker.inner.journal.lock().await.save(record).unwrap();
    (root, worker, spec, state, request)
}

async fn fixture() -> (
    tempfile::TempDir,
    Worker,
    WorkflowSpec,
    WorkflowState,
    RerunRequest,
) {
    fixture_with_count(1).await
}

async fn seed_intent(worker: &Worker, state: &WorkflowState, request: &RerunRequest) {
    let mut journal = worker.inner.journal.lock().await;
    let mut record = journal.records[&state.workflow_id].clone();
    record.lifecycle.todo_reruns.insert(
        request.request_id.clone(),
        Control {
            request: request.clone(),
            phase: "stopping".into(),
            accepted_at: now_ms(),
            error: None,
            generation: None,
            config: Some(json!(Config::default())),
        },
    );
    journal.save(record).unwrap();
}

#[tokio::test]
async fn recovery_finishes_each_durable_rerun_checkpoint_exactly_once() {
    for checkpoint in 0..3 {
        let home = tempfile::tempdir().unwrap();
        let _scope = opencoder_core::config::scoped_config_home(home.path().into());
        let (_root, worker, spec, state, request) = fixture().await;
        let guard = worker.inner.admission.lock().await;
        seed_intent(&worker, &state, &request).await;
        if checkpoint >= 1 {
            park(&worker, &spec, state.clone(), &request).await.unwrap();
        }
        if checkpoint >= 2 {
            let (_, parked) = persistence::load(&worker.inner.state.store, &state.workflow_id)
                .await
                .unwrap()
                .unwrap();
            let next = rerun::apply(&spec, parked, &request).unwrap();
            persistence::commit(
                &worker.inner.state.store,
                &spec,
                &next,
                "workflow_rerun_applied",
                json!({"request":request}),
            )
            .await
            .unwrap();
        }
        // The scheduler may resume after any one of the durable writes.
        recover_locked(&worker).await.unwrap();
        recover_locked(&worker).await.unwrap();
        let saved = worker.inner.journal.lock().await.records[&state.workflow_id].clone();
        assert_eq!(saved.assignment.index.status, ExecutionStatus::Pending);
        assert!(saved.queue.as_ref().unwrap().resume);
        let receipt = &saved.lifecycle.todo_reruns[&request.request_id];
        assert_eq!(receipt.phase, "queued");
        assert!(receipt.config.is_none());
        assert!(receipt.receipt().get("config").is_none());
        let (_, next) = persistence::load(&worker.inner.state.store, &state.workflow_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(next.world_epoch, 1);
        let events = worker
            .inner
            .state
            .store
            .todo_events_page(&state.workflow_id, 0, 100, 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(
            events
                .events
                .iter()
                .filter(|e| e.kind == "workflow_rerun_applied")
                .count(),
            1
        );
        // Re-entering recovery must not enqueue another copy.
        recover_locked(&worker).await.unwrap();
        assert_eq!(
            worker.inner.journal.lock().await.records[&state.workflow_id]
                .queue
                .as_ref()
                .unwrap()
                .sequence,
            saved.queue.unwrap().sequence
        );
        drop(guard);
        worker.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn cancellation_wins_over_a_pending_rerun_intent() {
    let home = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::config::scoped_config_home(home.path().into());
    let (_root, worker, spec, state, request) = fixture().await;
    let guard = worker.inner.admission.lock().await;
    seed_intent(&worker, &state, &request).await;
    park(&worker, &spec, state.clone(), &request).await.unwrap();
    crate::operations::durable_stop(&worker, &state.workflow_id, StopIntent::Cancel)
        .await
        .unwrap();
    recover_locked(&worker).await.unwrap();
    let saved = worker.inner.journal.lock().await.records[&state.workflow_id].clone();
    assert_eq!(
        saved.lifecycle.todo_reruns[&request.request_id].phase,
        "failed"
    );
    assert!(saved.lifecycle.todo_reruns[&request.request_id]
        .error
        .as_ref()
        .unwrap()
        .contains("cancelled"));
    assert!(saved.queue.is_none());
    assert_eq!(
        persistence::load(&worker.inner.state.store, &state.workflow_id)
            .await
            .unwrap()
            .unwrap()
            .1
            .world_epoch,
        0
    );
    drop(guard);
    worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancel_before_store_checkpoint_does_not_restart_a_completed_execution() {
    let home = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::config::scoped_config_home(home.path().into());
    let (_root, worker, _spec, state, request) = fixture().await;
    let guard = worker.inner.admission.lock().await;
    seed_intent(&worker, &state, &request).await;
    crate::operations::durable_stop(&worker, &state.workflow_id, StopIntent::Cancel)
        .await
        .unwrap();
    recover_locked(&worker).await.unwrap();
    let saved = worker.inner.journal.lock().await.records[&state.workflow_id].clone();
    assert_eq!(saved.assignment.index.status, ExecutionStatus::Done);
    assert_eq!(
        saved.lifecycle.todo_reruns[&request.request_id].phase,
        "failed"
    );
    assert!(saved.queue.is_none());
    assert_eq!(
        persistence::load(&worker.inner.state.store, &state.workflow_id)
            .await
            .unwrap()
            .unwrap()
            .1
            .generation,
        state.generation
    );
    drop(guard);
    worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn review_pages_all_nodes_and_rejects_a_stale_page_generation() {
    let home = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::config::scoped_config_home(home.path().into());
    // Definitions are frozen when created; transitions cannot replace their TODO IDs.
    let (_root, worker, spec, state, request) = fixture_with_count(111).await;
    {
        let mut journal = worker.inner.journal.lock().await;
        let mut record = journal.records[&state.workflow_id].clone();
        for (id, accepted_at) in [("z-older", 1), ("a-newer", 2)] {
            let mut request = request.clone();
            request.request_id = id.into();
            record.lifecycle.todo_reruns.insert(
                id.into(),
                Control {
                    request,
                    phase: "queued".into(),
                    error: None,
                    accepted_at,
                    generation: Some(1),
                    config: None,
                },
            );
        }
        journal.save(record).unwrap();
    }
    let mut next = opencoder_todos::domain::initial_state(
        &spec,
        state.workflow_id.clone(),
        state.parent_session_id.clone(),
    );
    next.generation = state.generation + 1;
    persistence::commit(
        &worker.inner.state.store,
        &spec,
        &next,
        "workflow_updated",
        json!({}),
    )
    .await
    .unwrap();
    let first =
        super::super::read::query(&worker, &state.workflow_id, json!({"section":"overview"}))
            .await
            .unwrap();
    assert_eq!(first.body["controls"][0]["request_id"], "z-older");
    assert_eq!(first.body["controls"][1]["request_id"], "a-newer");
    assert_eq!(first.body["total"], 111);
    assert_eq!(first.body["nodes"].as_array().unwrap().len(), 100);
    assert_eq!(first.body["next_ordinal"], 100);
    let second = super::super::read::query(
        &worker,
        &state.workflow_id,
        json!({"section":"overview","after_ordinal":100,"generation":next.generation}),
    )
    .await
    .unwrap();
    assert_eq!(second.body["nodes"].as_array().unwrap().len(), 11);
    assert!(second.body["next_ordinal"].is_null());
    let stale = super::super::read::query(
        &worker,
        &state.workflow_id,
        json!({"section":"overview","after_ordinal":100,"generation":state.generation}),
    )
    .await
    .unwrap();
    assert_eq!(stale.status, 409);
    worker.shutdown().await.unwrap();
}
