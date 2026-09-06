mod support;

use opencoder_core::fleet::{ExecutionIndex, ExecutionKind, ExecutionStatus};
use opencoder_node::fleet::NodeService;
use support::*;

fn index(id: &str, created_at: i64, node_id: &str) -> ExecutionIndex {
    ExecutionIndex {
        id: id.into(),
        created_at,
        kind: ExecutionKind::Agent,
        node_id: node_id.into(),
        status: ExecutionStatus::Idle,
    }
}

#[tokio::test]
async fn control_execution_list_uses_keyset_and_rejects_oversized_limits() {
    let fleet = Fleet::new(1, mock()).await;
    let node = fleet.nodes[0].registration().id;
    for row in [
        index("agent-d", 9, &node),
        index("agent-c", 10, &node),
        index("agent-a", 11, &node),
        index("agent-b", 11, &node),
    ] {
        fleet.state.fleet.put_index(&row).await.unwrap();
    }
    let first = fleet
        .call("GET", "/api/executions?limit=2", serde_json::Value::Null)
        .await;
    assert_eq!(first.status, 200, "{first:?}");
    assert_eq!(first.body["executions"][0]["id"], "agent-a");
    assert_eq!(first.body["executions"][1]["id"], "agent-b");
    let created_at = first.body["next_cursor"]["created_at"].as_i64().unwrap();
    let id = first.body["next_cursor"]["id"].as_str().unwrap();
    let second = fleet
        .call(
            "GET",
            &format!("/api/executions?limit=2&cursor_created_at={created_at}&cursor_id={id}"),
            serde_json::Value::Null,
        )
        .await;
    assert_eq!(second.status, 200, "{second:?}");
    assert_eq!(second.body["executions"][0]["id"], "agent-c");
    assert_eq!(second.body["executions"][1]["id"], "agent-d");
    assert!(second.body.get("next_cursor").is_none());

    let too_large = fleet
        .call("GET", "/api/executions?limit=501", serde_json::Value::Null)
        .await;
    assert_eq!(too_large.status, 400);
    fleet.shutdown().await;
}
