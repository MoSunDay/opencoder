//! `/api/agents/nfs` contract tests: thin router + oneshot (same shape as
//! `web_agents.rs`). The live-server slot is process-global (a static in
//! `api_agent_nfs`, mirroring `ACTIVATE_GATE`), so the whole lifecycle —
//! initial GET, start, reuse, stop, idempotence, spawn failure — runs
//! inside ONE test to stay deterministic. Config comes from the test
//! workdir's `opencoder.json`: `agent.nfs.port = 0` forces an ephemeral
//! port (the 2049 default may be taken) and `agent.agents_dir` pins the
//! export root to a tempdir.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::Router;
use tower::ServiceExt;

use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_store::{LibsqlStore, Store};

fn app(state: Arc<opencoder_web::AppState>) -> Router {
    Router::new()
        .route(
            "/api/agents/nfs",
            get(opencoder_web::api_agent_nfs::get_status)
                .post(opencoder_web::api_agent_nfs::post_set),
        )
        .with_state(state)
}

/// State whose workdir carries an `opencoder.json` steering the NFS opts:
/// ephemeral port + the given export root.
async fn state_with_config(agents_dir: &std::path::Path) -> Arc<opencoder_web::AppState> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let workdir = std::env::temp_dir().join(format!("oc-web-nfs-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workdir).unwrap();
    std::fs::write(
        workdir.join("opencoder.json"),
        serde_json::json!({
            "agent": {
                "agents_dir": agents_dir,
                "nfs": { "port": 0 },
            }
        })
        .to_string(),
    )
    .unwrap();
    Arc::new(opencoder_web::AppState {
        client_override: Some(Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>),
        brain: opencoder_web::api_brain::mock_brain(store.clone()),
        store,
        workdir,
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        team: opencoder_web::team_state::mock(),
        project: opencoder_web::ProjectService::new(),
    })
}

async fn call(
    app: Router,
    method: &str,
    uri: &str,
    body: impl Into<Option<serde_json::Value>>,
) -> (StatusCode, serde_json::Value) {
    let body = body.into();
    let req = Request::builder().method(method).uri(uri);
    let req = match body {
        Some(v) => req
            .header("content-type", "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => req.body(Body::empty()).unwrap(),
    };
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let v = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, v)
}

#[tokio::test]
async fn nfs_lifecycle_start_reuse_stop_and_failure() {
    let export_root = tempfile::tempdir().unwrap();
    let state = state_with_config(export_root.path()).await;
    let router = app(state.clone());

    // GET initial ⇒ documented stopped defaults.
    let (status, v) = call(router.clone(), "GET", "/api/agents/nfs", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["ok"], true);
    assert_eq!(v["status"]["running"], false);
    assert_eq!(v["status"]["port"], 2049);
    assert_eq!(v["status"]["export_root"], "");

    // POST enabled:true ⇒ started, running, ephemeral port resolved > 0.
    let (status, v) = call(
        router.clone(),
        "POST",
        "/api/agents/nfs",
        Some(serde_json::json!({ "enabled": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "start: {v}");
    assert_eq!(v["ok"], true);
    assert_eq!(v["started"], true);
    assert_eq!(v["status"]["running"], true);
    let port = v["status"]["port"].as_u64().expect("numeric port");
    assert!(port > 0, "ephemeral port must resolve, got {port}");

    // POST enabled:true again ⇒ same handle reused (not respawned).
    let (status, v) = call(
        router.clone(),
        "POST",
        "/api/agents/nfs",
        Some(serde_json::json!({ "enabled": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["started"], false);
    assert_eq!(v["status"]["running"], true);
    assert_eq!(
        v["status"]["port"].as_u64(),
        Some(port),
        "port must be reused"
    );

    // GET while running ⇒ live snapshot.
    let (status, v) = call(router.clone(), "GET", "/api/agents/nfs", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["status"]["running"], true);
    assert_eq!(v["status"]["port"].as_u64(), Some(port));

    // POST enabled:false ⇒ stopped shape.
    let (status, v) = call(
        router.clone(),
        "POST",
        "/api/agents/nfs",
        Some(serde_json::json!({ "enabled": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["ok"], true);
    assert_eq!(v["started"], false);
    assert_eq!(v["status"]["running"], false);

    // Idempotent stop.
    let (status, v) = call(
        router.clone(),
        "POST",
        "/api/agents/nfs",
        Some(serde_json::json!({ "enabled": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["ok"], true);
    assert_eq!(v["status"]["running"], false);

    // Spawn failure surfaces as 500 and leaves GET consistent: point the
    // export root at a path that does not exist.
    let state2 = state_with_config(&state.workdir.join("no-such-agents-root")).await;
    let router2 = app(state2);
    let (status, v) = call(
        router2.clone(),
        "POST",
        "/api/agents/nfs",
        Some(serde_json::json!({ "enabled": true })),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "failure: {v}");
    assert_eq!(v["ok"], false);
    assert!(v["error"].as_str().unwrap_or("").contains("nfs"), "{v}");
    let (status, v) = call(router2, "GET", "/api/agents/nfs", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["status"]["running"], false);
}
