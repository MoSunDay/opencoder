use super::*;
use crate::{lifecycle::Lifecycle, WorkerOptions};
use opencoder_core::{brain::layered::*, fleet::*};
use std::sync::Arc;

fn assignment(id: &str, kind: ExecutionKind, node: &str, input: Value) -> Assignment {
    Assignment {
        private_context: None,
        runtime: None,
        codex: None,
        index: ExecutionIndex {
            id: id.into(),
            kind,
            node_id: node.into(),
            status: ExecutionStatus::Pending,
            created_at: 1,
        },
        request: CreateExecution {
            id: id.into(),
            kind,
            target: None,
            node_id: None,
            input,
        },
        definition: None,
    }
}
fn record(assignment: Assignment, metadata: Value) -> Record {
    Record {
        assignment,
        annotations: json!({"brain_workspace":metadata}),
        queue: None,
        result: Value::Null,
        error: None,
        events: vec![],
        lifecycle: Lifecycle::default(),
    }
}
fn request() -> LayeredRequest {
    serde_json::from_value(json!({"schema_version":7,"inputs":{"issue":"frozen"},"plan":{
        "schema_version":7,"title":"workflow","objective":"verified repair","shared_workspace":true,"adjacent_only":true,
        "nodes":[{"node_id":"repair","layer_id":"repair","title":"repair","objective":"fix","capability_id":"op"}],
        "layers":[{"layer_id":"repair","title":"repair","task":"fix","objective":"fix","success_criteria":"fixed"}]
    }})).unwrap()
}

#[tokio::test]
async fn workspace_admission_shares_files_and_preserves_activation_identity() {
    let temp = tempfile::tempdir().unwrap();
    let _home = opencoder_core::config::scoped_config_home(temp.path().join("home"));
    let worker = Worker::open(
        WorkerOptions {
            name: "shared".into(),
            workdir: temp.path().join("work"),
            data_dir: temp.path().join("data"),
            workflow_root: None,
            max_runs: Some(1),
            dag: false,
        },
        Some(Arc::new(opencoder_llm::MockChatClient::new())),
    )
    .await
    .unwrap();
    let req = request();
    let root = assignment(
        "brain-shared",
        ExecutionKind::Brain,
        &worker.inner.registration.id,
        json!({"layered_request":req}),
    );
    let metadata = prepare(&worker, &root).await.unwrap();
    let root_record = record(root.clone(), metadata.clone());
    worker
        .inner
        .journal
        .lock()
        .await
        .save(root_record.clone())
        .unwrap();
    let mut change = opencoder_brain::layered::initialize("brain-shared", &req, 1).unwrap();
    change.run.layer = 1;
    change.run.round = 2;
    change.run.activation = 2;
    let operation = |id: &str, activation| LayeredOperation {
        run_id: "brain-shared".into(),
        operation_id: format!("op-{activation}"),
        execution_id: id.into(),
        execution_kind: ExecutionKind::Operator,
        node_id: "repair".into(),
        capability_id: "op".into(),
        layer: 1,
        round: activation,
        activation: activation.into(),
        attempt: 1,
        status: LayeredOperationStatus::Creating,
        source_sequence: None,
        cancel_requested: false,
    };
    change.operations = vec![
        operation("operator-first", 1),
        operation("operator-second", 2),
    ];
    worker
        .inner
        .state
        .store
        .commit_brain_layered(&change)
        .await
        .unwrap();
    let child = |id: &str, activation| {
        assignment(
            id,
            ExecutionKind::Operator,
            &worker.inner.registration.id,
            json!({"brain_workspace":true,"brain_layered":{"run_id":"brain-shared","operation_id":format!("op-{activation}"),
        "activation":activation,"node_id":"repair"},"layered_inputs":{"issue":"frozen"}}),
        )
    };
    let first = child("operator-first", 1);
    let first_meta = prepare(&worker, &first).await.unwrap();
    let first_record = record(first.clone(), first_meta.clone());
    worker
        .inner
        .journal
        .lock()
        .await
        .save(first_record.clone())
        .unwrap();
    for (input, status) in [
        (json!({"brain_workspace":{"root":"/tmp"}}), 400),
        (json!({"reviewed":true}), 200),
    ] {
        let reply = crate::operations::handle(
            &worker,
            NodeOperation::Command {
                execution: first.index.execution_ref(),
                command: ExecutionCommand {
                    action: "annotate".into(),
                    input,
                },
            },
        )
        .await
        .unwrap();
        assert_eq!(reply.status, status);
        let journal = worker.inner.journal.lock().await;
        assert_eq!(
            journal.records[&first.index.id].annotations["brain_workspace"],
            first_meta
        );
    }
    let first_dir = workdir(&first_record).unwrap().unwrap();
    std::fs::write(first_dir.join("evidence.txt"), "original").unwrap();
    let root_path = resolve(&first_record).unwrap().unwrap();
    std::fs::write(root_path.join("source.txt"), "fixed revision").unwrap();
    let second = child("operator-second", 2);
    let second_record = record(second.clone(), prepare(&worker, &second).await.unwrap());
    assert_ne!(
        workdir(&first_record).unwrap(),
        workdir(&second_record).unwrap()
    );
    assert_eq!(
        resolve(&first_record).unwrap(),
        resolve(&second_record).unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(resolve(&second_record).unwrap().unwrap().join("source.txt"))
            .unwrap(),
        "fixed revision"
    );
    assert_eq!(prepare(&worker, &first).await.unwrap(), first_meta);
    assert_eq!(
        std::fs::read_to_string(first_dir.join("evidence.txt")).unwrap(),
        "original"
    );
    assert!(envs(&first_record)
        .unwrap()
        .contains_key("OPENCODER_RUN_CONTEXT"));
    assert!(prepare(&worker, &child("operator-forged", 1))
        .await
        .is_err());
    let mut other_node = second.clone();
    other_node.index.node_id = "another-node".into();
    assert!(prepare(&worker, &other_node).await.is_err());
    let mut changed = root;
    changed.request.input["layered_request"]["inputs"]["issue"] = json!("mutated");
    assert!(prepare(&worker, &changed).await.is_err());
}

#[tokio::test]
async fn recovered_terminal_notice_commits_through_worker_without_resuming() {
    use opencoder_node::fleet::NodeService;

    let temp = tempfile::tempdir().unwrap();
    let _home = opencoder_core::config::scoped_config_home(temp.path().join("home"));
    let worker = Worker::open(
        WorkerOptions {
            name: "shared-receipt".into(),
            workdir: temp.path().join("work"),
            data_dir: temp.path().join("data"),
            workflow_root: None,
            max_runs: Some(1),
            dag: false,
        },
        Some(Arc::new(opencoder_llm::MockChatClient::new())),
    )
    .await
    .unwrap();
    let id = "brain-shared-receipt";
    let req = request();
    let root = assignment(
        id,
        ExecutionKind::Brain,
        &worker.inner.registration.id,
        json!({"schema_version":7,"layered_request":req}),
    );
    let metadata = prepare(&worker, &root).await.unwrap();
    worker
        .inner
        .journal
        .lock()
        .await
        .save(record(root, metadata))
        .unwrap();
    let mut initial = opencoder_brain::layered::initialize(id, &req, 1).unwrap();
    initial.run.phase = LayeredPhase::Paused;
    initial.run.layer = 1;
    initial.run.activation = 1;
    let operation = LayeredOperation {
        run_id: id.into(),
        operation_id: "op-current".into(),
        execution_id: "operator-current".into(),
        execution_kind: ExecutionKind::Operator,
        node_id: "repair".into(),
        capability_id: "op".into(),
        layer: 1,
        round: 1,
        activation: 1,
        attempt: 1,
        status: LayeredOperationStatus::Error,
        source_sequence: Some(4),
        cancel_requested: false,
    };
    initial.operations.push(operation.clone());
    let before = worker
        .inner
        .state
        .store
        .commit_brain_layered(&initial)
        .await
        .unwrap();
    let notice = LayeredTerminalEvent {
        run_id: id.into(),
        operation_id: operation.operation_id.clone(),
        execution_kind: operation.execution_kind,
        execution_id: operation.execution_id.clone(),
        status: LayeredOperationStatus::Done,
        source_sequence: 6,
    };
    let reply = worker
        .handle(NodeOperation::Brain {
            execution: ExecutionRef {
                id: id.into(),
                kind: ExecutionKind::Brain,
            },
            action: "layered_terminal".into(),
            input: json!(notice),
        })
        .await;
    assert_eq!(reply.status, 200, "{reply:?}");
    let after = worker
        .inner
        .state
        .store
        .brain_layered(id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.run.phase, LayeredPhase::Paused);
    assert_eq!(after.run.generation, before.run.generation + 1);
    assert_eq!(after.operations[0].status, LayeredOperationStatus::Done);
    assert_eq!(after.operations[0].source_sequence, Some(6));
    assert_eq!(after.run.layer, before.run.layer);
    assert_eq!(after.run.round, before.run.round);
    assert_eq!(after.run.max_rounds, before.run.max_rounds);
    assert_eq!(after.run.valid_layers, before.run.valid_layers);
    assert_eq!(after.run.activation, before.run.activation);
    let events = worker
        .inner
        .state
        .store
        .brain_layered_events(id, before.run.last_event_seq, 10)
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].source_sequence, Some(6));
    worker.shutdown().await.unwrap();
}

#[test]
fn workspace_paths_reject_parent_traversal_and_symlinks() {
    let temp = tempfile::tempdir().unwrap();
    assert!(checked(temp.path(), &temp.path().join("../outside")).is_err());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/tmp", temp.path().join("link")).unwrap();
        assert!(checked(temp.path(), &temp.path().join("link/file")).is_err());
    }
}
