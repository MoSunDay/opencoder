use crate::{journal::Record, lifecycle::Lifecycle, Worker, WorkerOptions};
use opencoder_core::fleet::*;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn pending_wasm_reservation_already_freezes_its_empty_resource_namespace() {
    let root = tempfile::tempdir().unwrap();
    let _config = opencoder_core::config::scoped_config_home(root.path().join("config"));
    let worker = Worker::open(
        WorkerOptions {
            name: "atomic-empty-resources".into(),
            workdir: root.path().join("work"),
            data_dir: root.path().join("node"),
            workflow_root: None,
            max_runs: Some(1),
            dag: false,
        },
        None,
    )
    .await
    .unwrap();
    for (id, kind, empty) in [
        ("wasm", json!({"type":"wasm","command":"probe.wasm"}), true),
        (
            "agent",
            json!({"type":"agent","prompt":"use resources"}),
            false,
        ),
    ] {
        let assignment = Assignment {
            runtime: None,
            codex: None,
            definition: Some(json!({"name":id,"steps":[{"name":"work","kind":kind}]})),
            index: ExecutionIndex {
                id: id.into(),
                created_at: 1,
                kind: ExecutionKind::Dag,
                node_id: worker.inner.registration.id.clone(),
                status: ExecutionStatus::Pending,
            },
            request: CreateExecution {
                id: id.into(),
                kind: ExecutionKind::Dag,
                target: None,
                input: json!({}),
                node_id: None,
            },
        };
        let prepared = super::admission::preparation::begin(&worker, assignment.clone())
            .unwrap()
            .unwrap();
        assert_eq!(prepared.definition, assignment.definition);
        let execution = worker
            .inner
            .layout
            .execution_dir(ExecutionKind::Dag, id)
            .unwrap();
        let resources = worker
            .inner
            .layout
            .resources_dir(ExecutionKind::Dag, id)
            .unwrap();
        assert!(execution.join("pending-create.json").is_file());
        assert!(
            !execution.join("execution.json").exists(),
            "not yet accepted"
        );
        assert_eq!(resources.is_dir(), empty);
        if empty {
            assert_eq!(std::fs::read_dir(&resources).unwrap().count(), 0);
            // Even a changed definition on replay cannot replace the original
            // frozen input or make the empty namespace consult a new pool.
            let mut changed = assignment.clone();
            changed.definition = Some(json!({"name":"changed","steps":[]}));
            let replay = super::admission::preparation::begin(&worker, changed)
                .unwrap()
                .unwrap();
            assert_eq!(replay.definition, assignment.definition);
            assert_eq!(
                crate::resources::pin(Some(&root.path().join("missing")), &resources).unwrap(),
                Some(resources)
            );
        }
    }
    worker.shutdown().await.unwrap();
}

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
