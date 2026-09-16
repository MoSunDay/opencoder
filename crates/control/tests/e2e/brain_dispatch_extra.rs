use crate::support::Harness;
use reqwest::Method;
use serde_json::{json, Value};
fn version() -> Value {
    json!({"id":"v2-test","version":1,"created_at":1,"changelog":"v2","plan":serde_json::from_str::<Value>(include_str!("../../../../examples/brain/repair-loop.json")).unwrap()})
}
#[tokio::test]
async fn v2_versions_pin_registered_definitions_and_reject_unregistered_targets() {
    let h = Harness::new().await;
    let mut unknown = version();
    unknown["plan"]["instances"][0]["capability_id"] = json!("unknown");
    let (status, body) = h
        .req(Method::POST, "/api/brain/plan-defs", Some(unknown))
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(body.to_string().contains("unregistered"));
    let (status, body) = h
        .req(Method::POST, "/api/brain/plan-defs", Some(version()))
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(body["version"]["plan"]["instances"][0]["action"]["definition"].is_object());
    let (status, _) = h
        .req(Method::GET, "/api/brain/plan-defs/v2-test/versions/1", None)
        .await;
    assert_eq!(status, 200);
    let (status, library) = h.req(Method::GET, "/api/brain/library", None).await;
    assert_eq!(status, 200);
    assert!(library["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["kind"] == "operator"));
    let mut legacy = version();
    legacy["plan"]["schema_version"] = json!(1);
    let (status, body) = h
        .req(Method::POST, "/api/brain/plan-defs", Some(legacy))
        .await;
    assert_eq!(status, 400);
    assert!(body.to_string().contains("migration"));
}

#[tokio::test]
async fn generic_execution_cannot_bypass_registered_brain_plan_admission() {
    let h = Harness::new().await;
    let (status, body) = h.req(Method::POST, "/api/executions", Some(json!({"id":"brain-bypass","kind":"brain","input":{"schema_version":2,"mode":"fixed","objective":"bypass","plan":version()}}))).await;
    assert_eq!(status, 409, "{body}");
    assert!(body.to_string().contains("/api/brain/runs"));
    assert!(h.state.fleet.index("brain-bypass").await.unwrap().is_none());
}
