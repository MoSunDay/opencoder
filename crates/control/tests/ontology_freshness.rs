//! HTTP contract for the freshness run trigger. Pure helper units live in
//! `src/ontology_freshness/planning_tests.rs` because the module is
//! crate-private; this suite drives the public surface end to end, with a
//! fleet that has no nodes so submission failures must stay item-local.
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use serde_json::{json, Value};
use std::time::Duration;
use tower::ServiceExt;

struct Fixture {
    // Underscored: the directory only needs to outlive the fixture.
    _root: tempfile::TempDir,
    state: std::sync::Arc<opencoder_control::AppState>,
    app: Router,
}

impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let work = root.path().join("work");
        let files = root.path().join("files");
        std::fs::create_dir(&work).unwrap();
        std::fs::write(
            work.join("opencoder.json"),
            json!({"ontology":{"files_dir":files,"nfs":{"port":0}}}).to_string(),
        )
        .unwrap();
        let state =
            opencoder_control::new_state(work, root.path().join("data"), None)
                .await
                .unwrap();
        let app = opencoder_control::build_app(state.clone(), None, false);
        Self {
            _root: root,
            state,
            app,
        }
    }

    async fn call(&self, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .body(if body.is_null() {
                Body::empty()
            } else {
                Body::from(body.to_string())
            })
            .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 8 * 1024 * 1024)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| json!({"raw": String::from_utf8_lossy(&bytes).to_string()})),
        )
    }

    async fn ok(&self, method: &str, path: &str, body: Value) -> Value {
        let (status, value) = self.call(method, path, body).await;
        assert!(status.is_success(), "{method} {path}: {status} {value}");
        value
    }

    async fn seeded_policy(&self) -> String {
        let kind = self.ok(
            "POST",
            "/api/ontology/envs/debug/entity-types",
            json!({"key":"dataset","name":"datasets"}),
        )
        .await["item"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        for name in ["alpha", "beta"] {
            self.ok(
                "POST",
                "/api/ontology/envs/debug/entities",
                json!({
                    "request_id": uuid::Uuid::new_v4(),
                    "entity_type_id": kind,
                    "name": name,
                    "source": "# source",
                    "ext": "# extra",
                    "attributes": {}
                }),
            )
            .await;
        }
        self.ok(
            "POST",
            "/api/ontology/envs/debug/freshness/policies",
            json!({
                "name": "nightly refresh",
                "description": "refresh datasets",
                "entity_type_id": kind,
                "executor": {
                    "kind": "agent",
                    "target": "agent://refresher",
                    "prompt_template": "refresh {{entity.name}}"
                },
                "concurrency": 5
            }),
        )
        .await["item"]["id"]
            .as_str()
            .unwrap()
            .to_string()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.state.lifecycle.retire();
    }
}

#[tokio::test]
async fn triggering_a_run_settles_items_gracefully_without_nodes() {
    let f = Fixture::new().await;
    let policy = f.seeded_policy().await;
    let trigger = format!("/api/ontology/envs/debug/freshness/policies/{policy}/runs");
    let (status, accepted) = f.call("POST", &trigger, Value::Null).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{status} {accepted}");
    let run = &accepted["item"];
    assert_eq!(run["status"], "queued");
    assert_eq!(run["policy_id"], json!(policy));
    assert_eq!(
        run["stats"],
        json!({"total": 2, "pending": 2, "dispatched": 0, "succeeded": 0, "failed": 0})
    );
    let run_id = run["id"].as_str().unwrap().to_string();
    let read = format!("/api/ontology/envs/debug/freshness/runs/{run_id}");

    // The spawned driver settles the run even though no node can accept the
    // submissions: every item fails with its own bounded error.
    let settled = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let run = f.ok("GET", &read, Value::Null).await["item"].clone();
            if matches!(run["status"].as_str(), Some("succeeded" | "partial" | "failed")) {
                break run;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .expect("run must settle without nodes");
    assert_eq!(settled["status"], "failed", "{settled}");
    let items = settled["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    for item in items {
        assert_eq!(item["status"], "failed", "{item}");
        assert_eq!(item["execution_id"], serde_json::Value::Null);
        let error = item["error"].as_str().unwrap();
        assert!(error.starts_with("submit "), "{item}");
        assert!(error.chars().count() <= 300, "{item}");
    }
    assert_eq!(
        settled["stats"],
        json!({"total": 2, "pending": 0, "dispatched": 0, "succeeded": 0, "failed": 2})
    );
}

#[tokio::test]
async fn triggering_with_an_unknown_policy_fails_before_creating_a_run() {
    let f = Fixture::new().await;
    let missing = uuid::Uuid::new_v4();
    let trigger = format!("/api/ontology/envs/debug/freshness/policies/{missing}/runs");
    let (status, body) = f.call("POST", &trigger, Value::Null).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{status} {body}");
    let runs = f
        .ok("GET", "/api/ontology/envs/debug/freshness/runs", Value::Null)
        .await;
    assert_eq!(runs["items"].as_array().unwrap().len(), 0);
}
