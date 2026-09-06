#[path = "../support/mod.rs"]
mod support;

use opencoder_core::fleet::{
    ExecutionCommand, ExecutionKind, ExecutionRef, ExecutionStatus, NodeOperation,
};
use opencoder_node::fleet::NodeService;
use opencoder_store::{LibsqlStore, SessionMeta, Store};
use serde_json::json;

#[tokio::test]
async fn internal_session_is_running_only_while_its_loop_is_live() {
    let dir = tempfile::tempdir().unwrap();
    let worker = support::worker(dir.path(), support::mock()).await;
    let top_id = "agent-top-idle";
    assert_eq!(
        worker
            .handle(NodeOperation::Create {
                assignment: support::assignment(
                    &worker,
                    top_id,
                    ExecutionKind::Agent,
                    json!({"prompt":""}),
                    None,
                ),
            })
            .await
            .status,
        200
    );
    assert_eq!(
        support::settled(&worker, top_id).await["execution"]["status"],
        "idle"
    );
    let id = "session-owned-child";
    let store = LibsqlStore::open(dir.path().join("node/runtime.db"))
        .await
        .unwrap();
    store
        .create_session(&SessionMeta {
            id: id.into(),
            title: Some("child".into()),
            agent: Some("act".into()),
            model: None,
            autopilot_mode: None,
            workdir_hash: None,
            created_at: 1,
            updated_at: 1,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: Some("subagent".into()),
            requirement: None,
        })
        .await
        .unwrap();

    let status = |indexes: &[opencoder_core::fleet::ExecutionIndex]| {
        indexes.iter().find(|index| index.id == id).unwrap().status
    };
    assert_eq!(
        status(&worker.indexes().await.unwrap()),
        ExecutionStatus::Done
    );
    let child = ExecutionRef {
        id: id.into(),
        kind: ExecutionKind::Agent,
    };
    assert_eq!(
        worker
            .handle(NodeOperation::Command {
                execution: child.clone(),
                command: ExecutionCommand {
                    action: "resume".into(),
                    input: json!({}),
                },
            })
            .await
            .status,
        404,
        "an internal session cannot be resumed as a top-level execution"
    );
    assert_eq!(
        worker
            .handle(NodeOperation::Command {
                execution: child,
                command: ExecutionCommand {
                    action: "prompt".into(),
                    input: json!({"prompt":"outside owner"}),
                },
            })
            .await
            .status,
        400,
        "an internal session cannot accept public top-level input"
    );
    let loop_guard = opencoder_session::loop_registry::LoopGuard::enter(id);
    assert_eq!(
        status(&worker.indexes().await.unwrap()),
        ExecutionStatus::Running
    );
    drop(loop_guard);
    assert_eq!(
        status(&worker.indexes().await.unwrap()),
        ExecutionStatus::Done
    );
    let top_status = |indexes: &[opencoder_core::fleet::ExecutionIndex]| {
        indexes
            .iter()
            .find(|index| index.id == top_id)
            .unwrap()
            .status
    };
    assert_eq!(
        top_status(&worker.indexes().await.unwrap()),
        ExecutionStatus::Idle
    );
    worker.drain_shutdown().await.unwrap();
    assert_eq!(
        top_status(&worker.indexes().await.unwrap()),
        ExecutionStatus::Interrupted
    );
    assert_eq!(
        status(&worker.indexes().await.unwrap()),
        ExecutionStatus::Done
    );
}
