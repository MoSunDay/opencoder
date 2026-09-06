mod support;

use opencoder_core::fleet::{ExecutionKind, NodeOperation};
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use support::*;

#[tokio::test]
async fn list_filters_by_stored_kind_and_routes_with_typed_reference() {
    let fleet = Fleet::new(1, mock()).await;
    let created = fleet
        .call(
            "POST",
            "/api/executions",
            json!({
                "id": "agent-index-contract",
                "kind": "agent",
                "input": {"prompt": "check typed routing"}
            }),
        )
        .await;
    assert_eq!(created.status, 202, "{created:?}");
    let _ = settled(&fleet.nodes[0], "agent-index-contract").await;

    let agents = fleet
        .call("GET", "/api/executions?kind=agent", Value::Null)
        .await;
    assert_eq!(agents.status, 200, "{agents:?}");
    let records = agents.body["executions"].as_array().unwrap();
    let record = records
        .iter()
        .find(|record| record["id"] == "agent-index-contract")
        .unwrap();
    assert_eq!(record["kind"], "agent");

    let teams = fleet
        .call("GET", "/api/executions?kind=team", Value::Null)
        .await;
    assert!(teams.body["executions"].as_array().unwrap().is_empty());
    assert_eq!(
        fleet
            .call("GET", "/api/executions?kind=unknown", Value::Null)
            .await
            .status,
        400
    );

    let index = fleet.nodes[0]
        .indexes()
        .await
        .unwrap()
        .into_iter()
        .find(|index| index.id == "agent-index-contract")
        .unwrap();
    assert_eq!(index.kind, ExecutionKind::Agent);
    let detail = fleet.nodes[0]
        .handle(NodeOperation::Inspect {
            execution: index.execution_ref(),
        })
        .await;
    assert_eq!(detail.status, 200, "{detail:?}");
    fleet.shutdown().await;
}
