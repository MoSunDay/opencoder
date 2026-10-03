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
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn rejected_dispatch_keeps_diagnostics_and_replay_cannot_duplicate_the_failure() {
    let (_scope, _home) = support::isolated_config();
    let directory = tempfile::tempdir().unwrap();
    let node = support::worker(directory.path(), Arc::new(client::LayeredClient::new())).await;
    let id = "brain-rejected-dispatch";
    let accepted = node
        .handle(NodeOperation::Create {
            assignment: support::assignment(
                &node,
                id,
                ExecutionKind::Brain,
                json!({"schema_version":7,"layered_request":plan::request(id)}),
                Some(json!({})),
            ),
        })
        .await;
    assert_eq!(accepted.status, 200, "{accepted:?}");
    support::settled(&node, id).await;
    control::decide_next_layer(&node, id).await;
    let frames = control::wait_dispatch(&node, 1).await;
    let operation = plan::operation(&frames[0]);
    for status in [503, 429] {
        let reply = control::ok(&node, id, "layered_receipt", json!({
            "operation_id":operation.operation_id,"reply":{"status":status,"body":{"error":"temporary"}}
        })).await;
        assert_eq!(reply["retry"], true);
        assert_eq!(
            control::snapshot(&node, id).await.operations[0].status,
            LayeredOperationStatus::Creating
        );
    }
    let receipt = json!({"operation_id":operation.operation_id,
        "reply":{"status":422,"body":{"error":"input patch: output path /revision missing"}}});
    control::ok(&node, id, "layered_receipt", receipt.clone()).await;
    let state = control::snapshot(&node, id).await;
    assert_eq!(state.run.phase, LayeredPhase::Ready);
    assert_eq!(state.operations[0].status, LayeredOperationStatus::Error);
    assert_eq!(state.operations[0].source_sequence, Some(0));
    let duplicate = control::ok(&node, id, "layered_receipt", receipt).await;
    assert_eq!(duplicate["duplicate"], true);
    assert_eq!(control::snapshot(&node, id).await, state);
    let events = control::events(&node, id).await;
    let failures: Vec<_> = events
        .iter()
        .filter(|e| e["event_type"] == "operation_terminal")
        .collect();
    assert_eq!(failures.len(), 1);
    assert!(failures[0]["reason_summary"]
        .as_str()
        .unwrap()
        .contains("/revision"));
    node.shutdown().await.unwrap();
}
