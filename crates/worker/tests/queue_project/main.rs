#[path = "../support/mod.rs"]
mod support;
use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use std::sync::{atomic::Ordering, Arc};
use support::*;

async fn create_project(node: &opencoder_worker::Worker, todo: &str) -> RpcReply {
    let mut a = assignment(
        node,
        &format!("project-{todo}"),
        ExecutionKind::Project,
        json!({"action":"plan","run_id":format!("prun-{todo}")}),
        Some(project_snapshot(todo)),
    );
    a.request.target = Some(todo.into());
    node.handle(NodeOperation::Create { assignment: a }).await
}

#[tokio::test]
async fn project_queue_preserves_pending_receipts_and_cancel_before_start() {
    let root = tempfile::tempdir().unwrap();
    let client = Arc::new(InterruptClient {
        calls: Default::default(),
        first_release: Default::default(),
        resumed_release: Default::default(),
    });
    let node = worker_with_client(root.path(), client.clone()).await;
    let configured = node
        .handle(NodeOperation::Maintenance {
            command: ExecutionCommand {
                action: "configure_scheduling".into(),
                input: json!({"max_runs":1,"queue_order":"fifo"}),
            },
        })
        .await;
    assert_eq!(configured.status, 200);
    let a = assignment(
        &node,
        "agent-holder",
        ExecutionKind::Agent,
        json!({"prompt":"hold","title":"queue capacity holder"}),
        None,
    );
    assert_eq!(
        node.handle(NodeOperation::Create { assignment: a })
            .await
            .status,
        200
    );
    for todo in ["cancel-queued", "run-queued"] {
        let accepted = create_project(&node, todo).await;
        assert_eq!(accepted.status, 200, "{accepted:?}");
        assert_eq!(accepted.body["status"], "pending");
        let execution = ExecutionRef {
            id: format!("project-{todo}"),
            kind: ExecutionKind::Project,
        };
        for action in ["project-receipt", "plan"] {
            let retry = node
                .handle(NodeOperation::Command {
                    execution: execution.clone(),
                    command: ExecutionCommand {
                        action: action.into(),
                        input: if action == "project-receipt" {
                            json!({"action":"plan","input":{"run_id":format!("prun-{todo}")}})
                        } else {
                            json!({"run_id":format!("prun-{todo}")})
                        },
                    },
                })
                .await;
            assert_eq!(retry.status, 200, "{retry:?}");
            assert_eq!(retry.body["status"], "pending", "{retry:?}");
        }
    }
    let indexes = node.indexes().await.unwrap();
    assert_eq!(
        indexes
            .iter()
            .find(|r| r.id == "prun-run-queued")
            .unwrap()
            .status,
        ExecutionStatus::Pending
    );
    let cancelled = node
        .handle(NodeOperation::Command {
            execution: ExecutionRef {
                id: "project-cancel-queued".into(),
                kind: ExecutionKind::Project,
            },
            command: ExecutionCommand {
                action: "cancel".into(),
                input: Value::Null,
            },
        })
        .await;
    assert_eq!(cancelled.status, 200, "{cancelled:?}");
    assert_eq!(cancelled.body["status"], "cancelled");
    assert_eq!(
        node.indexes()
            .await
            .unwrap()
            .iter()
            .find(|r| r.id == "prun-cancel-queued")
            .unwrap()
            .status,
        ExecutionStatus::Cancelled
    );
    client.first_release.notify_one();
    settled(&node, "agent-holder").await;
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while client.calls.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    client.resumed_release.notify_one();
    assert_eq!(
        settled(&node, "project-run-queued").await["execution"]["status"],
        "idle"
    );
    assert_eq!(
        client.calls.load(Ordering::SeqCst),
        2,
        "cancelled Project must never start a model request"
    );
    node.shutdown().await.unwrap();
}
