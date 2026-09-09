use super::{
    inspect,
    inspect::{classify_todo_workflow, pre_store_todo_workflow_view, TodoWorkflowView},
    *,
};
use crate::{
    journal::Record,
    lifecycle::{Lifecycle, StopIntent, TodoInitialization},
    WorkerOptions,
};
use opencoder_llm::MockChatClient;
use opencoder_store::{SessionMeta, TodoEventRecord, TodoWorkflowRecord};
use std::sync::Arc;

async fn worker() -> (tempfile::TempDir, Worker) {
    let root = tempfile::tempdir().unwrap();
    let workdir = root.path().join("work");
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    std::fs::write(workdir.join(".opencoder/ap.json"), r#"{"mode":"off"}"#).unwrap();
    let worker = Worker::open(
        WorkerOptions {
            name: "query-test".into(),
            workdir,
            data_dir: root.path().join("node"),
            workflow_root: None,
            max_runs: Some(1),
            dag: false,
        },
        Some(Arc::new(MockChatClient::new())),
    )
    .await
    .unwrap();
    (root, worker)
}

fn record(worker: &Worker, id: &str, status: ExecutionStatus) -> Record {
    let node_id = worker.inner.registration.id.clone();
    Record {
        queue: None,
        assignment: Assignment {
            codex: None,
            index: ExecutionIndex {
                id: id.into(),
                created_at: 1,
                kind: ExecutionKind::Todos,
                node_id: node_id.clone(),
                status,
            },
            request: CreateExecution {
                id: id.into(),
                kind: ExecutionKind::Todos,
                target: None,
                input: json!({}),
                node_id: Some(node_id),
            },
            definition: Some(json!({"schema_version":1,"id":id,"todos":[]})),
        },
        result: Value::Null,
        error: None,
        events: vec![],
        lifecycle: Lifecycle::accepted_todo(),
    }
}

fn execution(id: &str) -> ExecutionRef {
    ExecutionRef {
        id: id.into(),
        kind: ExecutionKind::Todos,
    }
}

async fn insert_workflow(worker: &Worker, id: &str) {
    let parent_id = format!("parent-{id}");
    worker
        .inner
        .state
        .store
        .create_session(&SessionMeta {
            id: parent_id.clone(),
            created_at: 1,
            updated_at: 1,
            ..SessionMeta::default()
        })
        .await
        .unwrap();
    worker
        .inner
        .state
        .store
        .create_todo_workflow(
            &TodoWorkflowRecord {
                id: id.into(),
                parent_session_id: parent_id,
                status: "running".into(),
                spec_json: json!({"id":id}),
                state_json: json!({"status":"running"}),
                generation: 1,
                created_at: 1,
                updated_at: 1,
                terminal_reason: None,
            },
            &[],
            &TodoEventRecord {
                seq: None,
                workflow_id: id.into(),
                kind: "workflow_created".into(),
                payload: json!({}),
                ts: 1,
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn accepted_todo_is_visible_before_and_after_workflow_initialization() {
    let (_root, worker) = worker().await;
    let id = "todos-query-initializing";
    worker
        .inner
        .journal
        .lock()
        .await
        .save(record(&worker, id, ExecutionStatus::Running))
        .unwrap();

    let initializing = inspect(&worker, &execution(id)).await.unwrap();
    assert_eq!(initializing.status, 200, "{initializing:?}");
    assert_eq!(initializing.body["execution"]["status"], "running");
    assert!(initializing.body["workflow"].is_null());
    assert_eq!(initializing.body["workflow_initializing"], true);

    insert_workflow(&worker, id).await;
    let committed_before_marker = inspect(&worker, &execution(id)).await.unwrap();
    assert_eq!(committed_before_marker.status, 200);
    assert_eq!(
        committed_before_marker.body["workflow_initialization"],
        "initializing"
    );
    assert_eq!(committed_before_marker.body["workflow_initializing"], true);
    worker
        .inner
        .journal
        .lock()
        .await
        .mark_todo_initialized(id)
        .unwrap();
    let initialized = inspect(&worker, &execution(id)).await.unwrap();
    assert_eq!(initialized.status, 200, "{initialized:?}");
    assert_eq!(initialized.body["workflow"]["workflow"]["id"], id);
    assert_eq!(initialized.body["workflow"]["items"], json!([]));
    assert_eq!(initialized.body["workflow_initialization"], "ready");
    assert!(initialized.body.get("workflow_initializing").is_none());

    let crashed_id = "todos-query-commit-before-marker-crash";
    worker
        .inner
        .journal
        .lock()
        .await
        .save(record(&worker, crashed_id, ExecutionStatus::Interrupted))
        .unwrap();
    insert_workflow(&worker, crashed_id).await;
    let recovered = inspect(&worker, &execution(crashed_id)).await.unwrap();
    assert_eq!(recovered.status, 200, "{recovered:?}");
    assert_eq!(recovered.body["execution"]["status"], "interrupted");
    assert_eq!(recovered.body["workflow_initialization"], "ready");
    assert_eq!(recovered.body["workflow"]["workflow"]["id"], crashed_id);
    worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn initialization_marker_preserves_a_concurrent_stop_intent() {
    let (_root, worker) = worker().await;
    let id = "todos-query-init-stop-race";
    worker
        .inner
        .journal
        .lock()
        .await
        .save(record(&worker, id, ExecutionStatus::Running))
        .unwrap();
    {
        let mut journal = worker.inner.journal.lock().await;
        journal
            .request_stop(id, StopIntent::Interrupt, true)
            .unwrap();
        journal.mark_todo_initialized(id).unwrap();
        let saved = &journal.records[id];
        assert_eq!(saved.assignment.index.status, ExecutionStatus::Cancelling);
        assert_eq!(saved.lifecycle.stop_intent, Some(StopIntent::Interrupt));
        assert_eq!(
            saved.lifecycle.todo_initialization,
            Some(TodoInitialization::Initialized)
        );
    }
    worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn initialized_execution_with_a_missing_workflow_is_a_consistency_error() {
    let (_root, worker) = worker().await;
    let id = "todos-query-missing";
    let mut record = record(&worker, id, ExecutionStatus::Running);
    record.lifecycle.todo_initialization = Some(TodoInitialization::Initialized);
    worker.inner.journal.lock().await.save(record).unwrap();
    let reply = inspect(&worker, &execution(id)).await.unwrap();
    assert_eq!(reply.status, 500, "{reply:?}");
    assert!(reply.body["error"]
        .as_str()
        .unwrap()
        .contains("workflow state is inconsistent"));
    worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn initialization_failure_and_preinit_stop_remain_inspectable() {
    let (_root, worker) = worker().await;
    let mut failed = record(&worker, "todos-query-init-failed", ExecutionStatus::Error);
    failed.error = Some("insert todo workflow: injected failure".into());
    worker.inner.journal.lock().await.save(failed).unwrap();

    let failed = inspect(&worker, &execution("todos-query-init-failed"))
        .await
        .unwrap();
    assert_eq!(failed.status, 200, "{failed:?}");
    assert_eq!(failed.body["workflow_initialization"], "failed");
    assert_eq!(failed.body["workflow_initializing"], false);
    assert!(failed.body["workflow"].is_null());
    assert!(failed.body["error"]
        .as_str()
        .unwrap()
        .contains("injected failure"));

    let mut stopped = record(
        &worker,
        "todos-query-init-stopped",
        ExecutionStatus::Interrupted,
    );
    stopped.lifecycle.stop_intent = Some(StopIntent::Interrupt);
    stopped.error = Some("explicit resume required".into());
    worker.inner.journal.lock().await.save(stopped).unwrap();
    let stopped = inspect(&worker, &execution("todos-query-init-stopped"))
        .await
        .unwrap();
    assert_eq!(stopped.status, 200, "{stopped:?}");
    assert_eq!(stopped.body["workflow_initialization"], "stopped");
    assert_eq!(stopped.body["execution"]["status"], "interrupted");
    assert!(stopped.body["workflow"].is_null());
    worker.shutdown().await.unwrap();
}

#[test]
fn workflow_lookup_errors_propagate_and_untracked_missing_rows_are_inconsistent() {
    assert!(matches!(
        pre_store_todo_workflow_view(Some(TodoInitialization::Accepted), ExecutionStatus::Running,),
        Some(TodoWorkflowView::Missing {
            state: "initializing",
            initializing: true,
        })
    ));
    assert!(pre_store_todo_workflow_view(
        Some(TodoInitialization::Initialized),
        ExecutionStatus::Running,
    )
    .is_none());

    let store_error = classify_todo_workflow(
        Err(anyhow::anyhow!("store unavailable")),
        Some(TodoInitialization::Accepted),
        ExecutionStatus::Running,
        None,
    )
    .err()
    .unwrap();
    assert!(store_error.to_string().contains("store unavailable"));

    assert!(matches!(
        classify_todo_workflow(Ok(None), None, ExecutionStatus::Running, None).unwrap(),
        TodoWorkflowView::Inconsistent
    ));
    assert!(matches!(
        classify_todo_workflow(
            Ok(None),
            Some(TodoInitialization::Accepted),
            ExecutionStatus::Error,
            None,
        )
        .unwrap(),
        TodoWorkflowView::Inconsistent
    ));
}
