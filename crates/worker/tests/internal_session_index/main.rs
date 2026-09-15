#[path = "../support/mod.rs"]
mod support;

use opencoder_core::fleet::{
    ExecutionCommand, ExecutionKind, ExecutionRef, ExecutionStatus, NodeOperation,
};
use opencoder_node::fleet::NodeService;
use opencoder_store::{LibsqlStore, SessionMeta, Store};
use serde_json::json;

#[tokio::test]
async fn index_replays_all_session_pages_with_activity_order_and_ties() {
    let (_guard, _home) = support::isolated_config();
    let dir = tempfile::tempdir().unwrap();
    let worker = support::worker(dir.path(), support::mock()).await;
    let store = LibsqlStore::open(dir.path().join("node/runtime.db"))
        .await
        .unwrap();
    let mut expected = std::collections::HashSet::new();
    for index in 0..1001 {
        let id = format!("agent-paged-{index:04}");
        expected.insert(id.clone());
        // Both page boundaries fall in a tied activity bucket. Half the rows
        // have an older/zero updated_at, and IDs differ from timestamp order.
        let activity = 100 + (index % 3);
        store
            .create_session(&SessionMeta {
                id,
                title: None,
                agent: Some("act".into()),
                model: None,
                autopilot_mode: None,
                workdir_hash: None,
                created_at: if index % 2 == 0 { activity } else { 1 },
                updated_at: if index % 2 == 0 { 0 } else { activity },
                summary: None,
                summary_seq: None,
                summary_images: vec![],
                handoff_seq: None,
                handoff_plan: None,
                skill: None,
                task_type: Some(if index % 2 == 0 { "parent" } else { "subagent" }.into()),
                requirement: None,
            })
            .await
            .unwrap();
    }
    for _ in 0..2 {
        let indexes = worker.indexes().await.unwrap();
        assert_eq!(indexes.len(), expected.len());
        assert_eq!(
            indexes
                .into_iter()
                .map(|row| row.id)
                .collect::<std::collections::HashSet<_>>(),
            expected
        );
    }
}

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
