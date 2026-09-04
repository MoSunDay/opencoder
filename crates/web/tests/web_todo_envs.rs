//! `/api/todo/envs` + `/api/todo/tools` REST contract tests over the share
//! tree. `set_share_dir_override` / `set_agents_dir_override` are
//! process-global, so every test serializes on `GATE` and points both roots
//! at fresh temp dirs (no restore needed: each test reinstalls fresh
//! overrides; other test binaries run in their own processes).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::{get, post};
use axum::Router;
use tower::ServiceExt;

use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_store::{LibsqlStore, Store};

static GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Lock the gate, then install fresh share/agents overrides. Keep the guard
/// alive for the whole test body.
async fn share() -> (
    tokio::sync::MutexGuard<'static, ()>,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    let guard = GATE.lock().await;
    let root = std::env::temp_dir().join(format!("oc-web-todo-envs-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let agents = std::env::temp_dir().join(format!("oc-web-todo-agents-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&agents).unwrap();
    opencoder_core::set_share_dir_override(Some(root.clone()));
    opencoder_core::agent::set_agents_dir_override(Some(agents.clone()));
    (guard, root, agents)
}

fn app(state: Arc<opencoder_web::AppState>) -> Router {
    Router::new()
        .route(
            "/api/todo/envs",
            get(opencoder_web::api_todo_envs::list_envs)
                .post(opencoder_web::api_todo_envs::create_env),
        )
        .route(
            "/api/todo/envs/:name",
            get(opencoder_web::api_todo_envs::get_env)
                .put(opencoder_web::api_todo_envs::update_env)
                .delete(opencoder_web::api_todo_envs::delete_env),
        )
        .route(
            "/api/todo/tools",
            get(opencoder_web::api_todo_envs::list_tools),
        )
        .route(
            "/api/todo/tools/import",
            post(opencoder_web::api_todo_envs::import_tool),
        )
        .with_state(state)
}

async fn state() -> Arc<opencoder_web::AppState> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let workdir = std::env::temp_dir().join(format!("oc-web-envs-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workdir).ok();
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
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
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
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, v)
}

/// E-1: create → get → list → delete roundtrip; dirs without context.json
/// are skipped by the listing.
#[tokio::test]
async fn env_crud_roundtrip() {
    let (_g, root, _agents) = share().await;
    let state = state().await;
    std::fs::create_dir_all(root.join("env").join("orphan")).unwrap();

    let (status, v) = call(
        app(state.clone()),
        "POST",
        "/api/todo/envs",
        Some(serde_json::json!({"name": "dev", "description": "开发环境"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["ok"], true);

    // Duplicate name conflicts even before any file content differs.
    let (status, v) = call(
        app(state.clone()),
        "POST",
        "/api/todo/envs",
        Some(serde_json::json!({"name": "dev"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");

    let (status, v) = call(app(state.clone()), "GET", "/api/todo/envs/dev", None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["name"], "dev");
    assert_eq!(v["description"], "开发环境");
    assert_eq!(v["tools"], serde_json::json!([]));
    assert_eq!(v["env_vars"], serde_json::json!({}));

    let (status, v) = call(app(state.clone()), "GET", "/api/todo/envs", None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let names: Vec<&str> = v["envs"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["name"].as_str())
        .collect();
    assert_eq!(
        names,
        vec!["dev"],
        "orphan dir without context.json skipped"
    );

    // Merge-patch: absent keys keep their value.
    let (status, v) = call(
        app(state.clone()),
        "PUT",
        "/api/todo/envs/dev",
        Some(serde_json::json!({"description": "新描述"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (_, v) = call(app(state.clone()), "GET", "/api/todo/envs/dev", None).await;
    assert_eq!(v["description"], "新描述");
    assert_eq!(v["tools"], serde_json::json!([]));

    let (status, v) = call(app(state.clone()), "DELETE", "/api/todo/envs/dev", None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (status, _) = call(app(state.clone()), "GET", "/api/todo/envs/dev", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(app(state.clone()), "DELETE", "/api/todo/envs/dev", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// E-2: a saved env can only reference tools that actually resolve — 400
/// before the file exists, 200 after the tool is seeded into the share.
#[tokio::test]
async fn update_env_validates_tool_refs() {
    let (_g, root, _agents) = share().await;
    let state = state().await;
    let (status, _) = call(
        app(state.clone()),
        "POST",
        "/api/todo/envs",
        Some(serde_json::json!({"name": "dev"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, v) = call(
        app(state.clone()),
        "PUT",
        "/api/todo/envs/dev",
        Some(serde_json::json!({"tools": ["/agent/tools/v3/ffmpeg"]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("工具引用无法解析"));

    let (status, v) = call(
        app(state.clone()),
        "PUT",
        "/api/todo/envs/dev",
        Some(serde_json::json!({"tools": ["not-a-ref"]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");

    std::fs::create_dir_all(root.join("agent").join("tools").join("v3")).unwrap();
    std::fs::write(root.join("agent/tools/v3/ffmpeg"), b"#!/bin/sh\n").unwrap();
    let (status, v) = call(
        app(state.clone()),
        "PUT",
        "/api/todo/envs/dev",
        Some(serde_json::json!({"tools": ["/agent/tools/v3/ffmpeg"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (_, v) = call(app(state.clone()), "GET", "/api/todo/envs/dev", None).await;
    assert_eq!(v["tools"], serde_json::json!(["/agent/tools/v3/ffmpeg"]));
}

/// E-3: tool listing unions the share copy and agent-bundled importables;
/// import copies bytes into the share and returns the canonical ref.
#[tokio::test]
async fn tools_listing_and_import() {
    let (_g, root, agents) = share().await;
    let state = state().await;
    std::fs::create_dir_all(root.join("agent").join("tools").join("v3")).unwrap();
    std::fs::write(root.join("agent/tools/v3/ffmpeg"), b"share copy").unwrap();
    let source = agents
        .join("myagent")
        .join("tools")
        .join("v3")
        .join("ffmpeg");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::write(&source, b"agent bundled").unwrap();
    // Non-version dirs under agent/tools must not surface as importables.
    std::fs::create_dir_all(agents.join("myagent/tools/active")).unwrap();

    let (status, v) = call(app(state.clone()), "GET", "/api/todo/tools", None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let tools = v["tools"].as_array().unwrap();
    let share_entry = tools
        .iter()
        .find(|t| t["ref"] == "/agent/tools/v3/ffmpeg" && t["source"] == "share")
        .expect("share entry listed");
    assert_eq!(share_entry.get("agent"), None);
    let importable = tools
        .iter()
        .find(|t| t["source"] == "importable")
        .expect("importable entry listed");
    assert_eq!(importable["ref"], "/agent/tools/v3/ffmpeg");
    assert_eq!(importable["agent"], "myagent");
    assert_eq!(importable["version"], "v3");
    assert_eq!(importable["tool"], "ffmpeg");
    assert_eq!(tools.len(), 2);

    let (status, v) = call(
        app(state.clone()),
        "POST",
        "/api/todo/tools/import",
        Some(serde_json::json!({"agent": "myagent", "version": "v3", "tool": "ffmpeg"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["ref"], "/agent/tools/v3/ffmpeg");
    let copied = std::fs::read(root.join("agent/tools/v3/ffmpeg")).unwrap();
    assert_eq!(copied, b"agent bundled", "import copies source bytes");

    let (status, v) = call(
        app(state.clone()),
        "POST",
        "/api/todo/tools/import",
        Some(serde_json::json!({"agent": "myagent", "version": "v3", "tool": "missing"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{v}");
    let (status, v) = call(
        app(state.clone()),
        "POST",
        "/api/todo/tools/import",
        Some(serde_json::json!({"agent": "../x", "version": "v3", "tool": "ffmpeg"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
}

/// E-4: traversal-shaped names are rejected at the body-validation path.
#[tokio::test]
async fn env_name_traversal_rejected() {
    let _g = share().await;
    let state = state().await;
    for name in ["a/b", "..", "", "x\\y"] {
        let (status, v) = call(
            app(state.clone()),
            "POST",
            "/api/todo/envs",
            Some(serde_json::json!({"name": name})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "name {name:?}: {v}");
    }
}
