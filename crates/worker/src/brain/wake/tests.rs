use super::*;
use crate::{journal::Record, lifecycle::Lifecycle, WorkerOptions};
use serde_json::json;

#[tokio::test]
async fn context_enqueue_while_recovery_waits_preserves_pending_activation() {
    let directory = tempfile::tempdir().unwrap();
    let _config = opencoder_core::config::scoped_config_home(directory.path().join("config"));
    let worker = Worker::open(
        WorkerOptions {
            name: "brain-recovery-race".into(),
            workdir: directory.path().join("work"),
            data_dir: directory.path().join("node"),
            workflow_root: None,
            max_runs: Some(1),
            dag: false,
        },
        Some(std::sync::Arc::new(opencoder_llm::MockChatClient::new())),
    )
    .await
    .unwrap();
    let admission = worker.inner.admission.lock().await;
    let id = "brain-context-race";
    let record = Record {
        assignment: Assignment {
            runtime: None,
            codex: None,
            index: ExecutionIndex {
                id: id.into(),
                kind: ExecutionKind::Brain,
                node_id: worker.inner.registration.id.clone(),
                created_at: 1,
                status: ExecutionStatus::Idle,
            },
            request: CreateExecution {
                id: id.into(),
                kind: ExecutionKind::Brain,
                target: None,
                node_id: None,
                input: json!({"schema_version":3}),
            },
            definition: Some(json!({})),
        },
        annotations: json!({}),
        queue: None,
        result: json!({}),
        error: None,
        events: vec![],
        lifecycle: Lifecycle::default(),
    };
    worker
        .inner
        .journal
        .lock()
        .await
        .save(record.clone())
        .unwrap();
    let gate = worker.lifecycle_gate(id).await;
    let guard = gate.lock().await;
    let recovery = recover_locked(&worker);
    tokio::pin!(recovery);
    assert!(futures::poll!(&mut recovery).is_pending());
    let accepted =
        crate::operations::queue::enqueue(&worker, record, worker.configuration().unwrap(), true)
            .await
            .unwrap();
    drop(guard);
    recovery.await.unwrap();
    let journal = worker.inner.journal.lock().await;
    let actual = &journal.records[id];
    assert_eq!(actual.assignment.index.status, ExecutionStatus::Pending);
    assert_eq!(
        actual.queue.as_ref().unwrap().sequence,
        accepted.queue.as_ref().unwrap().sequence
    );
    assert_eq!(
        worker
            .inner
            .pending_runs
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    drop(journal);
    // Keep the fixture's uninitialized root from being scheduled on shutdown.
    worker.inner.stopping.cancel();
    drop(admission);
    worker.shutdown().await.unwrap();
}
