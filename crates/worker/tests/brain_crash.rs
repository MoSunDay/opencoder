#![cfg(not(windows))]
#[path = "scheduler_v4/client.rs"]
mod client;
#[path = "scheduler_v4/control.rs"]
mod control;
#[path = "scheduler_v4/plan.rs"]
mod plan;
mod support;
use opencoder_core::{brain::layered::*, fleet::*};
use opencoder_node::fleet::NodeService;
use opencoder_worker::Worker;
use serde_json::{json, Value};
use std::sync::Arc;

async fn create(node: &Worker, id: &str) {
    let reply = node.handle(NodeOperation::Create {
        assignment: support::assignment(node, id, ExecutionKind::Brain,
            json!({"schema_version":7,"layered_request":plan::request(id),"frozen_capabilities":plan::catalog()}), Some(json!({}))),
    }).await;
    assert_eq!(reply.status, 200, "{reply:?}");
    support::settled(node, id).await;
}

#[tokio::test]
async fn unclean_worker_exit_fails_waiting_root_and_clean_restart_retains_it() {
    let (_scope, _home) = support::isolated_brain_config();
    for crashed in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let model = Arc::new(client::LayeredClient::new());
        let node = support::worker(directory.path(), model.clone()).await;
        let id = "brain-worker-exit";
        create(&node, id).await;
        control::decide_next_layer(&node, id).await;
        let before = control::wait_phase(&node, id, LayeredPhase::Waiting).await;
        node.shutdown().await.unwrap();
        drop(node);
        if crashed {
            std::fs::write(
                directory.path().join("node/worker-lifecycle.json"),
                r#"{"state":"running"}"#,
            )
            .unwrap();
        }
        let node = support::worker(directory.path(), model).await;
        let after = control::snapshot(&node, id).await;
        assert_eq!(
            after.run.phase,
            if crashed {
                LayeredPhase::Failed
            } else {
                LayeredPhase::Waiting
            }
        );
        assert_eq!(after.operations, before.operations);
        if crashed {
            assert_eq!(
                node.indexes()
                    .await
                    .unwrap()
                    .iter()
                    .find(|index| index.id == id)
                    .unwrap()
                    .status,
                ExecutionStatus::Error
            );
            let frames = control::frames(&node).await;
            for action in ["layered_wake", "layered_dispatch", "layered_guidance"] {
                assert!(control::actions(&frames, action).is_empty());
            }
            let reply = node
                .handle(NodeOperation::Brain {
                    execution: ExecutionRef {
                        id: id.into(),
                        kind: ExecutionKind::Brain,
                    },
                    action: "resume".into(),
                    input: Value::Null,
                })
                .await;
            assert!(reply.status >= 400);
            let generation = after.run.generation;
            node.shutdown().await.unwrap();
            drop(node);
            std::fs::write(
                directory.path().join("node/worker-lifecycle.json"),
                r#"{"state":"running"}"#,
            )
            .unwrap();
            let node =
                support::worker(directory.path(), Arc::new(client::LayeredClient::new())).await;
            assert_eq!(
                control::snapshot(&node, id).await.run.generation,
                generation
            );
            node.shutdown().await.unwrap();
        } else {
            assert!(
                !control::actions(&control::frames(&node).await, "layered_dispatch").is_empty()
            );
            node.shutdown().await.unwrap();
        }
    }
}

struct PanickingClient;
impl opencoder_llm::ChatStream for PanickingClient {
    fn chat_stream(
        &self,
        _: opencoder_llm::ChatRequest,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<opencoder_llm::LlmEvent>> {
        panic!("simulated unexpected decision task crash")
    }
}

#[tokio::test]
async fn decision_task_panic_fails_projection_and_journal_without_retry() {
    let (_scope, _home) = support::isolated_brain_config();
    let directory = tempfile::tempdir().unwrap();
    let node = support::worker(directory.path(), Arc::new(PanickingClient)).await;
    let id = "brain-panic";
    create(&node, id).await;
    control::decide_next_layer(&node, id).await;
    let failed = control::wait_phase(&node, id, LayeredPhase::Failed).await;
    assert!(failed.operations.is_empty());
    support::settled(&node, id).await;
    let record: Value = serde_json::from_slice(
        &std::fs::read(
            directory
                .path()
                .join(format!("node/brain/{id}/execution.json")),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(record["assignment"]["index"]["status"], "error");
    assert_eq!(
        record["annotations"]["layered_decision_attempt"]["attempt"],
        1
    );
    assert_eq!(record["result"]["phase"], "failed");
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn restart_settles_a_failure_committed_before_its_journal_without_another_event() {
    use opencoder_store::{LibsqlStore, Store};
    let (_scope, _home) = support::isolated_brain_config();
    let directory = tempfile::tempdir().unwrap();
    let node = support::worker(directory.path(), Arc::new(client::LayeredClient::new())).await;
    let id = "brain-partial-failure";
    create(&node, id).await;
    let before = control::snapshot(&node, id).await;
    node.shutdown().await.unwrap();
    drop(node);
    let store = LibsqlStore::open(directory.path().join("node/runtime.db"))
        .await
        .unwrap();
    let failed = store
        .commit_brain_layered(
            &opencoder_brain::layered::fail(&before, "Worker crashed".into(), 100).unwrap(),
        )
        .await
        .unwrap();
    drop(store);
    std::fs::write(
        directory.path().join("node/worker-lifecycle.json"),
        r#"{"state":"running"}"#,
    )
    .unwrap();
    let node = support::worker(directory.path(), Arc::new(client::LayeredClient::new())).await;
    assert_eq!(control::snapshot(&node, id).await, failed);
    let record: Value = serde_json::from_slice(
        &std::fs::read(
            directory
                .path()
                .join(format!("node/brain/{id}/execution.json")),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(record["assignment"]["index"]["status"], "error");
    assert_eq!(record["result"]["phase"], "failed");
    node.shutdown().await.unwrap();
}
