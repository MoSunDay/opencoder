use crate::{journal::Record, lifecycle::Lifecycle, Worker, WorkerOptions};
use opencoder_core::fleet::*;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn durable_replay_does_not_wait_for_an_unrelated_cold_admission() {
    let root = tempfile::tempdir().unwrap();
    let _config = opencoder_core::config::scoped_config_home(root.path().join("config"));
    let worker = Worker::open(
        WorkerOptions {
            name: "retry-with-busy-admission".into(),
            workdir: root.path().join("work"),
            data_dir: root.path().join("node"),
            workflow_root: None,
            max_runs: Some(1),
            dag: false,
        },
        Some(Arc::new(opencoder_llm::MockChatClient::new())),
    )
    .await
    .unwrap();
    let assignment = Assignment {
        runtime: None,
        codex: None,
        definition: None,
        index: ExecutionIndex {
            id: "agent-accepted".into(),
            created_at: 1,
            kind: ExecutionKind::Agent,
            node_id: worker.inner.registration.id.clone(),
            status: ExecutionStatus::Done,
        },
        request: CreateExecution {
            id: "agent-accepted".into(),
            kind: ExecutionKind::Agent,
            target: Some("act".into()),
            input: json!({"prompt":"original"}),
            node_id: None,
        },
    };
    worker
        .inner
        .journal
        .lock()
        .await
        .save(Record {
            assignment: assignment.clone(),
            annotations: Value::Null,
            queue: None,
            result: Value::Null,
            error: None,
            events: vec![],
            lifecycle: Lifecycle::default(),
        })
        .unwrap();
    let journal = root.path().join("node/agent/agent-accepted/execution.json");
    let before = std::fs::read(&journal).unwrap();
    let _busy = worker.inner.admission.lock().await;
    for (request, expected) in [
        (assignment.clone(), 200),
        (
            {
                let mut changed = assignment.clone();
                changed.request.input = json!({"prompt":"different"});
                changed
            },
            409,
        ),
        (
            {
                let mut wrong_owner = assignment.clone();
                wrong_owner.index.node_id = "other-node".into();
                wrong_owner
            },
            409,
        ),
    ] {
        let reply = tokio::time::timeout(
            Duration::from_millis(200),
            super::create::create(&worker, request),
        )
        .await
        .expect("durable replay waited for unrelated admission")
        .unwrap();
        assert_eq!(reply.status, expected, "{reply:?}");
        if expected == 200 {
            assert_eq!(reply.body, json!(assignment.index));
        }
    }
    assert_eq!(std::fs::read(journal).unwrap(), before);
    let mut fresh = assignment;
    fresh.index.id = "agent-new".into();
    fresh.request.id = fresh.index.id.clone();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(20),
            super::create::create(&worker, fresh)
        )
        .await
        .is_err(),
        "new acceptance must still serialize with the global admission gate"
    );
}
