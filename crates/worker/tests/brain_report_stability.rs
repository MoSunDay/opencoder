//! Reading the durable scheduler outbox must not wake the reporter itself.
mod support;

use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use serde_json::json;
use support::*;

#[tokio::test]
async fn unchanged_brain_report_does_not_trigger_another_report() {
    let (_scope, _home) = isolated_config();
    let dir = tempfile::tempdir().unwrap();
    let node = worker(dir.path(), mock()).await;
    let id = "brain-report-stable";
    let reply = node.handle(NodeOperation::Create {
        assignment: assignment(&node, id, ExecutionKind::Brain,
            json!({"schema_version":6,"layered_request":{"schema_version":6,"plan":{"schema_version":6,"title":"report","objective":"wait","nodes":[{"node_id":"work","title":"work","capability_ids":["builtin-agent-act"],"layer":1,"objective":"work","success_criteria":"verified"}]}}}), Some(json!({}))),
    }).await;
    assert_eq!(reply.status, 200, "{reply:?}");
    settled(&node, id).await;
    node.shutdown().await.unwrap();
    let mut changes = node.changes();
    changes.borrow_and_update();
    for _ in 0..3 {
        let report = node.report().await.unwrap();
        assert!(report.brain.iter().any(|frame| matches!(frame,
            NodeFrame::Brain { action, .. } if action == "layered_wake")));
        assert!(
            !changes.has_changed().unwrap(),
            "unchanged outbox reads must not cause a report loop"
        );
    }
    let cancelled = node
        .handle(NodeOperation::Brain {
            execution: ExecutionRef {
                id: id.into(),
                kind: ExecutionKind::Brain,
            },
            action: "cancel".into(),
            input: json!({}),
        })
        .await;
    assert_eq!(cancelled.status, 200, "{cancelled:?}");
    assert!(
        changes.has_changed().unwrap(),
        "a committed cancellation must be reported"
    );
    changes.borrow_and_update();
    for _ in 0..3 {
        let report = node.report().await.unwrap();
        assert!(report.brain.is_empty());
        assert!(
            !changes.has_changed().unwrap(),
            "settled terminal reports must stay quiet"
        );
    }
}
