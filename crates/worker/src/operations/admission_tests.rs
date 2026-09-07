use crate::{journal::Record, lifecycle::Lifecycle, Worker, WorkerOptions};
use opencoder_core::fleet::*;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn rejected_project_commands_never_change_durable_admission() {
    let dir = tempfile::tempdir().unwrap();
    let worker = Worker::open(
        WorkerOptions {
            name: "admission-test".into(),
            workdir: dir.path().join("work"),
            data_dir: dir.path().join("node"),
            workflow_root: None,
            max_runs: Some(1),
            dag: true,
        },
        Some(std::sync::Arc::new(opencoder_llm::MockChatClient::new())),
    )
    .await
    .unwrap();
    let id = "project-todo";
    let original = Record {
        assignment: Assignment {
            index: ExecutionIndex {
                id: id.into(),
                kind: ExecutionKind::Project,
                node_id: worker.inner.registration.id.clone(),
                created_at: 1,
                status: ExecutionStatus::Idle,
            },
            request: CreateExecution {
                id: id.into(),
                kind: ExecutionKind::Project,
                target: Some("todo".into()),
                node_id: None,
                input: json!({}),
            },
            definition: Some(
                json!({"todo":{"id":"todo","title":"task","draft":"original",
                "agent":"act","status":"draft","created_at":1,"updated_at":1},"goals":[],"milestones":[]}),
            ),
        },
        result: json!({"run_id":"previous"}),
        error: None,
        events: vec![],
        lifecycle: Lifecycle::default(),
    };
    worker
        .inner
        .journal
        .lock()
        .await
        .save(original.clone())
        .unwrap();
    let journal = dir.path().join("node/project/project-todo/execution.json");
    let before = std::fs::read(&journal).unwrap();
    let mut snapshot = original.assignment.definition.clone().unwrap();
    snapshot["todo"]["draft"] = json!("latest");
    let execution = ExecutionRef {
        id: id.into(),
        kind: ExecutionKind::Project,
    };
    let invoke = |snapshot: Value| {
        super::command::command(
            &worker,
            &execution,
            ExecutionCommand {
                action: "plan".into(),
                input: json!({"snapshot":snapshot}),
            },
        )
    };

    worker
        .inner
        .active
        .lock()
        .await
        .insert(id.into(), CancellationToken::new());
    assert_eq!(invoke(snapshot.clone()).await.unwrap().status, 409);
    assert_eq!(std::fs::read(&journal).unwrap(), before);
    // A known accepted ID has a valid empty event stream before session creation.
    let mut agent = original.clone();
    agent.assignment.index.id = "agent-starting".into();
    agent.assignment.index.kind = ExecutionKind::Agent;
    agent.assignment.request.id = "agent-starting".into();
    agent.assignment.request.kind = ExecutionKind::Agent;
    worker.inner.journal.lock().await.save(agent).unwrap();
    worker
        .inner
        .active
        .lock()
        .await
        .insert("agent-starting".into(), CancellationToken::new());
    let stream = super::query::events(
        &worker,
        &ExecutionRef {
            id: "agent-starting".into(),
            kind: ExecutionKind::Agent,
        },
        0,
    )
    .await
    .unwrap();
    assert_eq!(stream.status, 200);
    assert_eq!(stream.body["finished"], false);
    worker.inner.active.lock().await.clear();

    let slot = worker.inner.slots.clone().acquire_owned().await.unwrap();
    assert_eq!(invoke(snapshot.clone()).await.unwrap().status, 429);
    assert_eq!(std::fs::read(&journal).unwrap(), before);
    drop(slot);
    snapshot["todo"]["agent"] = json!("missing-resource-agent");
    let rejected = invoke(snapshot.clone()).await.unwrap();
    assert_eq!(rejected.status, 400, "{rejected:?}");
    assert_eq!(std::fs::read(&journal).unwrap(), before);
    assert!(
        !dir.path()
            .join("node/project/project-todo/resources")
            .exists(),
        "rejected preflight must not pin resources for a later retry"
    );
    snapshot["todo"]["id"] = json!("other");
    assert_eq!(invoke(snapshot).await.unwrap().status, 400);
    assert_eq!(std::fs::read(&journal).unwrap(), before);
    let rejected = super::command::command(
        &worker,
        &ExecutionRef {
            id: "agent-starting".into(),
            kind: ExecutionKind::Agent,
        },
        ExecutionCommand {
            action: "execute".into(),
            input: json!({}),
        },
    )
    .await
    .unwrap();
    assert_eq!(rejected.status, 400);
}

#[tokio::test]
async fn missing_runc_rootfs_is_rejected_before_durable_acceptance() {
    let dir = tempfile::tempdir().unwrap();
    let worker = Worker::open(
        WorkerOptions {
            name: "runc-preflight".into(),
            workdir: dir.path().join("work"),
            data_dir: dir.path().join("node"),
            workflow_root: None,
            max_runs: Some(1),
            dag: true,
        },
        Some(std::sync::Arc::new(opencoder_llm::MockChatClient::new())),
    )
    .await
    .unwrap();
    let id = "dag-rootfs";
    let reply = super::create::create(
        &worker,
        Assignment {
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
            definition: Some(json!({"name":"runc","steps":[{
                "name":"step","kind":{"type":"wasm","command":"tool.wasm", "sandbox":"runc"}
            }]})),
        },
    )
    .await
    .unwrap();
    assert_eq!(reply.status, 400, "{reply:?}");
    assert!(reply
        .body
        .to_string()
        .contains(&dir.path().join("node/dag/rootfs").display().to_string()));
    assert!(!dir
        .path()
        .join("node/dag/dag-rootfs/execution.json")
        .exists());
    assert!(!dir.path().join("node/dag/dag-rootfs/resources").exists());
}

#[tokio::test]
async fn create_never_adopts_an_unowned_execution_directory() {
    let dir = tempfile::tempdir().unwrap();
    let worker = Worker::open(
        WorkerOptions {
            name: "unowned-directory".into(),
            workdir: dir.path().join("work"),
            data_dir: dir.path().join("node"),
            workflow_root: None,
            max_runs: Some(1),
            dag: false,
        },
        Some(std::sync::Arc::new(opencoder_llm::MockChatClient::new())),
    )
    .await
    .unwrap();
    let id = "agent-unowned";
    std::fs::create_dir_all(dir.path().join("node/agent").join(id)).unwrap();
    let reply = super::create::create(
        &worker,
        Assignment {
            index: ExecutionIndex {
                id: id.into(),
                created_at: 1,
                kind: ExecutionKind::Agent,
                node_id: worker.inner.registration.id.clone(),
                status: ExecutionStatus::Pending,
            },
            request: CreateExecution {
                id: id.into(),
                kind: ExecutionKind::Agent,
                target: None,
                input: json!({}),
                node_id: None,
            },
            definition: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(reply.status, 409, "{reply:?}");
    assert!(!dir
        .path()
        .join("node/agent/agent-unowned/execution.json")
        .exists());
}

#[tokio::test]
async fn system_history_is_queryable_and_stoppable_but_cannot_restart() {
    let dir = tempfile::tempdir().unwrap();
    let worker = Worker::open(
        WorkerOptions {
            name: "retired-system".into(),
            workdir: dir.path().join("work"),
            data_dir: dir.path().join("node"),
            workflow_root: None,
            max_runs: Some(1),
            dag: false,
        },
        Some(std::sync::Arc::new(opencoder_llm::MockChatClient::new())),
    )
    .await
    .unwrap();
    assert!(!worker
        .inner
        .registration
        .kinds
        .contains(&ExecutionKind::System));
    let history = Record {
        assignment: Assignment {
            index: ExecutionIndex {
                id: "system-history".into(),
                created_at: 1,
                kind: ExecutionKind::System,
                node_id: worker.inner.registration.id.clone(),
                status: ExecutionStatus::Interrupted,
            },
            request: CreateExecution {
                id: "system-history".into(),
                kind: ExecutionKind::System,
                target: None,
                input: json!({"prompt":"legacy"}),
                node_id: None,
            },
            definition: Some(json!({"nodes":[]})),
        },
        result: Value::Null,
        error: Some("node stopped".into()),
        events: vec![],
        lifecycle: Lifecycle::default(),
    };
    worker
        .inner
        .journal
        .lock()
        .await
        .save(history.clone())
        .unwrap();
    let reference = history.assignment.index.execution_ref();
    assert_eq!(
        super::query::inspect(&worker, &reference)
            .await
            .unwrap()
            .status,
        200
    );
    assert_eq!(
        super::command::command(
            &worker,
            &reference,
            ExecutionCommand {
                action: "resume".into(),
                input: Value::Null,
            },
        )
        .await
        .unwrap()
        .status,
        400
    );
    assert_eq!(
        super::command::command(
            &worker,
            &reference,
            ExecutionCommand {
                action: "cancel".into(),
                input: Value::Null,
            },
        )
        .await
        .unwrap()
        .status,
        200
    );
    let inspected = super::query::inspect(&worker, &reference).await.unwrap();
    assert_eq!(inspected.body["execution"]["status"], "cancelled");

    let mut rejected = history.assignment;
    rejected.index.id = "system-new".into();
    rejected.request.id = "system-new".into();
    assert_eq!(
        super::create::create(&worker, rejected)
            .await
            .unwrap()
            .status,
        400
    );
    assert!(!dir
        .path()
        .join("node/system/system-new/execution.json")
        .exists());
}
