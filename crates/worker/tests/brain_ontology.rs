#[path = "support/mod.rs"]
mod support;
use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use support::*;

#[path = "support/brain.rs"]
mod graph_support;
use graph_support::{doc, plan, GraphClient};
use std::sync::Arc;

async fn wait_phase(fleet: &Fleet, id: &str, phase: &str) -> Value {
    let mut last = Value::Null;
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let reply = fleet
                .call("GET", &format!("/api/brain/runs/{id}"), Value::Null)
                .await;
            last = reply.body;
            if last["phase"] == phase {
                return;
            }
            if last["phase"] == "failed" {
                panic!("brain failed: {last}");
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(outcome.is_ok(), "did not reach {phase}: {last}");
    last
}

#[tokio::test]
async fn fixed_plan_executes_through_real_node_channels_and_returns_verified_outputs() {
    let _config = support::isolated_config();
    let client = Arc::new(GraphClient::default());
    let fleet = Fleet::new(2, client.clone()).await;
    let saved = fleet.call("POST", "/api/brain/plan-defs", plan()).await;
    assert_eq!(saved.status, 200, "{saved:?}");
    let body = json!({"id":"brain-integration","node_id":fleet.nodes[0].registration().id,"mode":"fixed","objective":"Review","plan":{"id":"plan-integration","version":1},"inputs":doc()});
    let created = fleet.call("POST", "/api/brain/runs", body.clone()).await;
    assert_eq!(created.status, 202, "{created:?}");
    let snapshot = wait_phase(&fleet, "brain-integration", "completed").await;
    assert_eq!(
        snapshot["deliverables"]
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap(),
        "node-owned answer"
    );
    assert_eq!(snapshot["plan"]["version"], 1);
    assert_eq!(snapshot["total_instances"], 1);
    let index = fleet
        .state
        .fleet
        .indexes(None, Some(ExecutionKind::Agent), 100)
        .await
        .unwrap();
    assert_eq!(index.len(), 1);
    let calls = client.requests.lock().unwrap().len();
    assert_eq!(
        fleet.call("POST", "/api/brain/runs", body).await.status,
        202
    );
    assert_eq!(client.requests.lock().unwrap().len(), calls);
    let events = fleet
        .call(
            "GET",
            "/api/brain/runs/brain-integration/events-page",
            Value::Null,
        )
        .await;
    assert_eq!(events.status, 200);
    assert!(!events.body["events"].as_array().unwrap().is_empty());
    assert!(snapshot["watermark"].as_i64().unwrap() > 0);
    fleet.shutdown().await;
}

#[tokio::test]
async fn input_wait_releases_slot_and_pause_fences_then_cancel_closes_root() {
    let _config = support::isolated_config();
    let fleet = Fleet::new(1, mock()).await;
    let definition = plan();
    assert_eq!(
        fleet
            .call("POST", "/api/brain/plan-defs", definition)
            .await
            .status,
        200
    );
    assert_eq!(fleet.call("POST","/api/brain/runs",json!({"id":"brain-input","node_id":fleet.nodes[0].registration().id,"mode":"fixed","objective":"Input run","plan":{"id":"plan-integration","version":1}})).await.status,202);
    wait_phase(&fleet, "brain-input", "waiting_input").await;
    assert_eq!(
        fleet
            .call(
                "POST",
                "/api/brain/runs/brain-input/commands",
                json!({"action":"pause"})
            )
            .await
            .status,
        200
    );
    assert_eq!(
        fleet
            .call(
                "POST",
                "/api/brain/runs/brain-input/inputs",
                json!({"name":"document","value":{"name":"Ada","markdown":"body"}})
            )
            .await
            .status,
        200
    );
    let snapshot = wait_phase(&fleet, "brain-input", "paused").await;
    assert!(snapshot["instances"]
        .as_array()
        .unwrap()
        .iter()
        .all(|i| i["execution"].is_null()));
    assert_eq!(
        fleet
            .call(
                "POST",
                "/api/brain/runs/brain-input/commands",
                json!({"action":"cancel"})
            )
            .await
            .status,
        200
    );
    wait_phase(&fleet, "brain-input", "cancelled").await;
    fleet.shutdown().await;
}

#[tokio::test]
async fn dynamic_plans_once_publishes_version_and_uses_registered_capabilities() {
    let _config = support::isolated_config();
    let client = Arc::new(GraphClient::default());
    let fleet = Fleet::new(1, client.clone()).await;
    let created=fleet.call("POST","/api/brain/runs",json!({"id":"brain-dynamic","node_id":fleet.nodes[0].registration().id,"mode":"dynamic","objective":"Review dynamically","inputs":doc()})).await;
    assert_eq!(created.status, 202, "{created:?}");
    let snapshot = wait_phase(&fleet, "brain-dynamic", "completed").await;
    assert_eq!(snapshot["plan"]["id"], "plan-dynamic");
    let requests = client.requests.lock().unwrap().clone();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.purpose == opencoder_llm::RequestPurpose::Planning)
            .count(),
        2
    );
    assert!(fleet
        .state
        .fleet
        .brain_plan_version("plan-dynamic", 1)
        .await
        .unwrap()
        .is_some());
    let library = fleet.call("GET", "/api/brain/library", Value::Null).await;
    assert!(library.body["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c.get("source_plan").is_none()));
    assert!(fleet
        .state
        .fleet
        .definition("brain_plan", "plan-dynamic")
        .await
        .unwrap()
        .unwrap()["stable_version"]
        .is_null());
    fleet.shutdown().await;
}
