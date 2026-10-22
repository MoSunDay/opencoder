use axum::{
    body::{to_bytes, Body},
    http::Request,
    Router,
};
use opencoder_core::{
    identity::{Identity, Role},
    ontology::platform_for_path,
};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;

#[path = "ontology_catalog/release.rs"]
mod release;

struct Fixture {
    root: tempfile::TempDir,
    state: Arc<opencoder_control::AppState>,
    app: Router,
}
impl Fixture {
    async fn new() -> Self {
        Self::with_background(true).await
    }
    async fn with_background(enabled: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let work = root.path().join("work");
        std::fs::create_dir(&work).unwrap();
        std::fs::create_dir(root.path().join("agents")).unwrap();
        std::fs::create_dir_all(root.path().join("share/todo")).unwrap();
        std::fs::write(
            work.join("opencoder.json"),
            json!({"agent":{
                "agents_dir":root.path().join("agents"),"share_dir":root.path().join("share")
            }})
            .to_string(),
        )
        .unwrap();
        let state = opencoder_control::new_state(work, root.path().join("data"), None)
            .await
            .unwrap();
        state
            .lifecycle
            .catalog_started
            .store(!enabled, std::sync::atomic::Ordering::SeqCst);
        let app = opencoder_control::build_app(state.clone(), None, false);
        Self { root, state, app }
    }
    async fn call(&self, role: Role, method: &str, path: &str, value: Value) -> (u16, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(value.to_string()))
            .unwrap();
        request.extensions_mut().insert(Identity {
            name: "catalog-test".into(),
            role,
        });
        let response = self.app.clone().oneshot(request).await.unwrap();
        let code = response.status().as_u16();
        let bytes = to_bytes(response.into_body(), 8 * 1024 * 1024)
            .await
            .unwrap();
        (
            code,
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| json!({"text":String::from_utf8_lossy(&bytes)})),
        )
    }
    async fn ok(&self, method: &str, path: &str, value: Value) -> Value {
        let (code, result) = self.call(Role::Admin, method, path, value).await;
        assert!(
            (200..300).contains(&code),
            "{method} {path}: {code} {result}"
        );
        result
    }
    async fn list(&self) -> Value {
        self.ok(
            "GET",
            "/api/ontology/envs/debug/capabilities?limit=500",
            Value::Null,
        )
        .await
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.state.lifecycle.retire();
    }
}

#[tokio::test]
async fn native_changes_automatically_update_catalog_and_keep_read_permissions() {
    let f = Fixture::new().await;
    let sync = "/api/ontology/envs/debug/sync";
    f.ok("POST", sync, Value::Null).await;
    let initial = f.list().await;
    assert!(initial["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["capability"]["key"] == "platform:project"));
    assert!(initial["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["capability"]["kind"] == "operator"));
    f.ok(
        "POST",
        "/api/teams",
        json!({"name":"catalog-team","captain":"act","members":[{"agent":"act"}]}),
    )
    .await;
    let item = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let list = f.list().await;
            if let Some(item) = list["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["capability"]["source_id"] == "team-catalog-team")
            {
                break item.clone();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("successful native write must trigger synchronization");
    assert!(item["capability"]["dependencies"]
        .as_array()
        .unwrap()
        .contains(&json!("native:builtin-agent-act")));
    assert_eq!(f.call(Role::Viewer, "GET", sync, Value::Null).await.0, 200);
    assert_eq!(f.call(Role::Viewer, "POST", sync, Value::Null).await.0, 403);
    assert_eq!(
        f.call(Role::Viewer, "GET", &format!("{sync}/check"), Value::Null)
            .await
            .0,
        200
    );
    let detail = f
        .ok(
            "GET",
            &format!(
                "/api/ontology/envs/debug/entities/{}",
                item["id"].as_str().unwrap()
            ),
            Value::Null,
        )
        .await;
    assert_eq!(detail["management"]["managed_by"], "platform-catalog");
    let filtered = f
        .ok(
            "GET",
            "/api/ontology/envs/debug/capabilities?kind=team&query=catalog-team&limit=1",
            Value::Null,
        )
        .await;
    assert_eq!(filtered["total"], 1);
    assert_eq!(filtered["items"][0]["id"], item["id"]);
}

#[tokio::test]
async fn source_failure_keeps_previous_snapshot_and_recovers_after_restart() {
    let f = Fixture::new().await;
    f.ok("POST","/api/dag/defs",json!({"name":"catalog-dag","steps":[{"name":"run","kind":{"type":"binary","resource":"tool@v1"}}]})).await;
    f.ok("POST", "/api/ontology/envs/debug/sync", Value::Null)
        .await;
    let before = f.list().await;
    let agents = f.root.path().join("agents");
    std::fs::remove_dir(&agents).unwrap();
    std::fs::write(&agents, "invalid source directory").unwrap();
    let failed = f
        .call(
            Role::Admin,
            "POST",
            "/api/ontology/envs/debug/sync",
            Value::Null,
        )
        .await;
    assert_eq!(failed.0, 502, "{}", failed.1);
    assert_eq!(f.list().await, before);
    let status = f
        .ok("GET", "/api/ontology/envs/debug/sync", Value::Null)
        .await;
    assert_eq!(status["state"], "error");
    assert!(status["last_success"].is_string());
    std::fs::remove_file(&agents).unwrap();
    std::fs::create_dir(&agents).unwrap();
    f.state.lifecycle.retire();
    f.state.lifecycle.drained().await;
    let reopened =
        opencoder_control::new_state(f.state.workdir.clone(), f.root.path().join("data"), None)
            .await
            .unwrap();
    let app = opencoder_control::build_app(reopened.clone(), None, false);
    let recovered = Fixture {
        root: tempfile::tempdir().unwrap(),
        state: reopened,
        app,
    };
    recovered
        .ok("POST", "/api/ontology/envs/debug/sync", Value::Null)
        .await;
    assert_eq!(recovered.list().await, before);
    recovered
        .ok("DELETE", "/api/dag/defs/catalog-dag", Value::Null)
        .await;
    recovered
        .ok("POST", "/api/ontology/envs/debug/sync", Value::Null)
        .await;
    assert!(!recovered.list().await["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["capability"]["source_id"] == "dag-catalog-dag"));
}

#[tokio::test]
async fn concurrent_syncs_are_serialized_and_retired_servers_do_not_start_more() {
    let f = Fixture::new().await;
    let path = "/api/ontology/envs/debug/sync";
    let (a, b) = tokio::join!(
        f.ok("POST", path, Value::Null),
        f.ok("POST", path, Value::Null)
    );
    assert_eq!(a["state"], "ready");
    assert_eq!(b["state"], "ready");
    let entries = f.list().await;
    let ids: std::collections::BTreeSet<_> = entries["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), entries["total"].as_u64().unwrap() as usize);
    f.state.lifecycle.retire();
    f.state.lifecycle.drained().await;
    assert_eq!(f.call(Role::Admin, "POST", path, Value::Null).await.0, 502);
}

#[tokio::test]
async fn forwarded_resource_changes_notify_sync_only_after_success() {
    let f = Fixture::with_background(false).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let resource_service = format!("http://{}", listener.local_addr().unwrap());
    let owner = Router::new().route(
        "/api/agents/nfs",
        axum::routing::post(|axum::Json(value): axum::Json<Value>| async move {
            let status = if value["enabled"] == true {
                axum::http::StatusCode::SERVICE_UNAVAILABLE
            } else {
                axum::http::StatusCode::OK
            };
            (status, axum::Json(json!({"enabled":false})))
        }),
    );
    let task = tokio::spawn(async move { axum::serve(listener, owner).await.unwrap() });
    f.state
        .lifecycle
        .platform
        .set(opencoder_core::fleet::release::PlatformConfig {
            release_id: "fixture".into(),
            state_dir: f.root.path().into(),
            host_service: resource_service.clone(),
            resource_service,
        })
        .unwrap();
    f.state.lifecycle.credential.set("fixture".into()).unwrap();
    f.ok("POST", "/api/agents/nfs", json!({"enabled":false}))
        .await;
    tokio::time::timeout(
        Duration::from_secs(1),
        f.state.lifecycle.catalog_changed.notified(),
    )
    .await
    .expect("forwarded success must notify the same background synchronizer");
    assert_eq!(
        f.call(
            Role::Admin,
            "POST",
            "/api/agents/nfs",
            json!({"enabled":true})
        )
        .await
        .0,
        503
    );
    assert!(tokio::time::timeout(
        Duration::from_millis(20),
        f.state.lifecycle.catalog_changed.notified(),
    )
    .await
    .is_err());
    task.abort();
}

#[test]
fn public_control_routes_have_platform_catalog_ownership() {
    fn visit(path: &std::path::Path, count: &mut usize) {
        for item in std::fs::read_dir(path).unwrap() {
            let path = item.unwrap().path();
            if path.is_dir() {
                visit(&path, count);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let source = std::fs::read_to_string(&path).unwrap();
                for route in source.split(".route(").skip(1) {
                    let route = route.trim_start();
                    if !route.starts_with('"') {
                        continue;
                    }
                    let Some(route) = route.split('"').nth(1) else {
                        continue;
                    };
                    if !route.starts_with("/api/") && route != "/metrics" {
                        continue;
                    }
                    assert!(
                        platform_for_path(route).is_some(),
                        "unregistered route {route} in {}",
                        path.display()
                    );
                    *count += 1;
                }
            }
        }
    }
    let mut count = 0;
    visit(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut count,
    );
    assert!(count > 100);
}
