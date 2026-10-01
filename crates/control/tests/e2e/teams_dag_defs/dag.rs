use reqwest::Method;
use serde_json::json;

use crate::support::Harness;

const SPEC: &str = r#"{"name":"etl-demo","steps":[
    {"name":"fetch","kind":{"type":"binary","resource":"tool"}},
    {"name":"load","depends_on":["fetch"],"kind":{"type":"binary","resource":"tool"}}]}"#;
#[tokio::test]
async fn dag_definitions_crud() {
    let h = Harness::new().await;
    let (status, body) = h
        .req(
            Method::POST,
            "/api/dag/defs",
            Some(json!({"spec": serde_json::from_str::<serde_json::Value>(SPEC).unwrap()})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], json!("etl-demo"));
    assert_eq!(body["name"], json!("etl-demo"));

    let (status, body) = h.req(Method::GET, "/api/dag/defs", None).await;
    assert_eq!(status, 200, "{body}");
    let defs = body.as_array().unwrap();
    assert_eq!(defs[0]["id"], json!("etl-demo"));
    assert_eq!(defs[0]["spec"]["steps"].as_array().unwrap().len(), 2);

    let (status, body) = h.req(Method::GET, "/api/dag/defs/etl-demo", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["spec"]["name"], json!("etl-demo"));
    let (status, body) = h.req(Method::GET, "/api/dag/defs/none", None).await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"], json!("DAG not found"));

    // Specs with dangling dependencies are rejected by domain validation.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/dag/defs",
            Some(json!({"spec": {"name": "bad", "steps": [
            {"name": "a", "depends_on": ["ghost"], "kind": {"type":"binary","resource":"tool"}}]}})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("depends on unknown step"),
        "{body}"
    );

    let (status, body) = h.req(Method::DELETE, "/api/dag/defs/etl-demo", None).await;
    assert_eq!(status, 200, "{body}");
    let (status, _) = h.req(Method::GET, "/api/dag/defs/etl-demo", None).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn dag_save_tracks_update_time_and_preserves_creation_time() {
    let h = Harness::new().await;
    let mut spec: serde_json::Value = serde_json::from_str(SPEC).unwrap();
    let before = opencoder_core::message::now_ms();
    let (status, created) = h
        .req(Method::POST, "/api/dag/defs", Some(json!({"spec": spec})))
        .await;
    assert_eq!(status, 200, "{created}");
    assert!(created["created_at"].as_i64().unwrap() >= before);
    assert_eq!(created["updated_at"], created["created_at"]);

    spec["description"] = json!("edited workflow");
    let (status, updated) = h
        .req(
            Method::POST,
            "/api/dag/defs",
            Some(json!({"spec": spec, "created_at": 1, "updated_at": 1})),
        )
        .await;
    assert_eq!(status, 200, "{updated}");
    assert_eq!(updated["id"], created["id"]);
    assert_eq!(updated["created_at"], created["created_at"]);
    assert!(updated["updated_at"].as_i64().unwrap() > created["updated_at"].as_i64().unwrap());
    assert_eq!(updated["spec"]["description"], "edited workflow");

    let (status, listed) = h.req(Method::GET, "/api/dag/defs", None).await;
    assert_eq!(status, 200, "{listed}");
    assert_eq!(listed, json!([updated]));
    let (status, fetched) = h.req(Method::GET, "/api/dag/defs/etl-demo", None).await;
    assert_eq!(status, 200, "{fetched}");
    assert_eq!(fetched, updated);

    spec["steps"] = json!([]);
    let (status, rejected) = h.req(Method::POST, "/api/dag/defs", Some(spec)).await;
    assert_eq!(status, 400, "{rejected}");
    let (status, unchanged) = h.req(Method::GET, "/api/dag/defs/etl-demo", None).await;
    assert_eq!(status, 200, "{unchanged}");
    assert_eq!(unchanged, updated);
}

#[tokio::test]
async fn legacy_dag_gets_timestamps_on_save_and_concurrent_edits_keep_creation() {
    let h = Harness::new().await;
    let spec: serde_json::Value = serde_json::from_str(SPEC).unwrap();
    h.state
        .fleet
        .put_definition(
            "dag",
            "etl-demo",
            &json!({"id":"etl-demo","name":"etl-demo","spec":spec}),
        )
        .await
        .unwrap();
    let before = opencoder_core::message::now_ms();
    let (first, second) = tokio::join!(
        h.req(Method::POST, "/api/dag/defs", Some(spec.clone())),
        h.req(Method::POST, "/api/dag/defs", Some(spec)),
    );
    assert_eq!(first.0, 200, "{}", first.1);
    assert_eq!(second.0, 200, "{}", second.1);
    assert!(first.1["created_at"].as_i64().unwrap() >= before);
    assert_eq!(first.1["created_at"], second.1["created_at"]);
    let first_time = first.1["updated_at"].as_i64().unwrap();
    let second_time = second.1["updated_at"].as_i64().unwrap();
    assert_ne!(first_time, second_time);
    let (status, stored) = h.req(Method::GET, "/api/dag/defs/etl-demo", None).await;
    assert_eq!(status, 200, "{stored}");
    assert_eq!(stored["created_at"], first.1["created_at"]);
    assert_eq!(stored["updated_at"], first_time.max(second_time));
}

fn binary_step(name: &str, depends_on: serde_json::Value) -> serde_json::Value {
    json!({"name": name, "depends_on": depends_on, "kind": {"type":"binary","resource":"tool"}})
}

/// save_dag rejects unparsable and invalid specs with the domain validator's
/// aggregated messages; a bare spec (no `{"spec": ...}` wrapper) is accepted,
/// and deleting an unknown definition is an idempotent success.
#[tokio::test]
async fn dag_definition_spec_validation_table() {
    let h = Harness::new().await;
    let table: Vec<(&str, serde_json::Value, &str)> = vec![
        (
            "steps wrong type",
            json!({"name": "bad", "steps": "nope"}),
            "invalid type",
        ),
        ("missing steps", json!({"name": "bad"}), "missing field"),
        (
            "empty steps",
            json!({"name": "bad", "steps": []}),
            "spec.steps must not be empty",
        ),
        (
            "duplicate step names",
            json!({"name": "bad", "steps": [binary_step("a", json!([])), binary_step("a", json!([]))]}),
            "duplicate step name",
        ),
        (
            "dependency cycle",
            json!({"name": "bad", "steps": [
                binary_step("a", json!(["b"])), binary_step("b", json!(["a"]))]}),
            "cycle detected",
        ),
    ];
    for (label, spec, expected) in table {
        let (status, body) = h
            .req(Method::POST, "/api/dag/defs", Some(json!({"spec": spec})))
            .await;
        assert_eq!(status, 400, "{label}: {body}");
        assert!(
            body["error"].as_str().is_some_and(|e| e.contains(expected)),
            "{label}: {body}"
        );
    }

    // Bare-spec body: the whole payload is the spec (unwrap_or branch).
    let (status, body) = h
        .req(
            Method::POST,
            "/api/dag/defs",
            Some(json!({"name": "bare-demo", "steps": [binary_step("only", json!([]))]})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], json!("bare-demo"));
    assert_eq!(body["spec"]["steps"].as_array().unwrap().len(), 1);
    let (status, body) = h.req(Method::GET, "/api/dag/defs/bare-demo", None).await;
    assert_eq!(status, 200, "{body}");

    // Deleting an unknown definition is a no-op success (pinned contract).
    let (status, body) = h.req(Method::DELETE, "/api/dag/defs/none", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({"ok": true}));
}
