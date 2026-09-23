//! A separate test process keeps the node's process-global change watch local
//! to this fixture, so another fleet cannot mask a self-triggering report.
mod support;
use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use serde_json::json;
use support::*;

#[tokio::test]
async fn reading_scheduler_outbox_does_not_trigger_another_node_report() {
    let (_config, _home) = isolated_config();
    let directory = tempfile::tempdir().unwrap();
    let node = worker(directory.path(), mock()).await;
    let id = "brain-report-no-feedback";
    let reference = ExecutionRef {
        id: id.into(),
        kind: ExecutionKind::Brain,
    };
    let reply = node
        .handle(NodeOperation::Create {
            assignment: assignment(
                &node,
                id,
                ExecutionKind::Brain,
                json!({"schema_version":6,"layered_request":{"schema_version":6,
                    "plan":{"schema_version":6,"title":"report","objective":"wait for a decision","nodes":[{"node_id":"work","title":"work","capability_ids":["builtin-agent-act"],"layer":1,"objective":"work","success_criteria":"verified"}]}}}),
                Some(json!({})),
            ),
        })
        .await;
    assert_eq!(reply.status, 200, "{reply:?}");
    settled(&node, id).await;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while node.snapshot().active_runs != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    for terminal in [false, true] {
        if terminal {
            let cancelled = node
                .handle(NodeOperation::Brain {
                    execution: reference.clone(),
                    action: "cancel".into(),
                    input: json!({}),
                })
                .await;
            assert_eq!(cancelled.status, 200, "{cancelled:?}");
        }
        let mut changes = node.changes();
        node.report().await.unwrap();
        changes.borrow_and_update();
        node.report().await.unwrap();
        assert!(
            !changes.has_changed().unwrap(),
            "reading a report must not enqueue itself"
        );
    }
    node.shutdown().await.unwrap();
}
