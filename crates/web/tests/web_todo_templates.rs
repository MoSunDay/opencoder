//! `/api/todo/templates` REST contract tests: spec validation on every
//! context write, metadata merge-patch, version lifecycle and env binding.
//! Same process-global override serialization as `web_todo_envs.rs`.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::{delete, get, post};
use axum::Router;
use tower::ServiceExt;

use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_store::{LibsqlStore, Store};
use opencoder_web::api_todo_template_versions as ver;
use opencoder_web::api_todo_templates as tpl;

static GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn share() -> (tokio::sync::MutexGuard<'static, ()>, std::path::PathBuf) {
    let guard = GATE.lock().await;
    let root = std::env::temp_dir().join(format!("oc-web-todo-tpl-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let agents =
        std::env::temp_dir().join(format!("oc-web-todo-tpl-agents-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&agents).unwrap();
    opencoder_core::set_share_dir_override(Some(root.clone()));
    opencoder_core::agent::set_agents_dir_override(Some(agents));
    (guard, root)
}

fn app(state: Arc<opencoder_web::AppState>) -> Router {
    Router::new()
        .route(
            "/api/todo/templates",
            get(tpl::list_templates).post(tpl::create_template),
        )
        .route(
            "/api/todo/templates/:name",
            get(tpl::get_template).delete(ver::delete_template),
        )
        .route(
            "/api/todo/templates/:name/todo.json",
            get(tpl::get_meta).put(tpl::update_meta),
        )
        .route(
            "/api/todo/templates/:name/new-version",
            post(ver::new_version),
        )
        .route(
            "/api/todo/templates/:name/:version/context.json",
            get(tpl::get_context).put(tpl::put_context),
        )
        .route(
            "/api/todo/templates/:name/:version/env.json",
            get(tpl::get_env_binding).put(tpl::put_env_binding),
        )
        .route(
            "/api/todo/templates/:name/:version",
            delete(ver::delete_version),
        )
        .with_state(state)
}

async fn state() -> Arc<opencoder_web::AppState> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let workdir = std::env::temp_dir().join(format!("oc-web-tpl-{}", uuid::Uuid::new_v4()));
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

/// Minimal valid single-TODO WorkflowSpec.
fn spec(agent: &str) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "id": "wf-1",
        "name": "demo",
        "objective": "ship it",
        "todos": [{
            "id": "t1", "title": "T1", "requirement_background": "bg", "instructions": "do it",
            "agent": agent, "acceptance": { "criteria": "c" },
        }],
        "metadata": {}
    })
}

/// Two-TODO spec with a dependency cycle — rejected by domain validation.
fn cycle_spec() -> serde_json::Value {
    let mut bad = spec("act");
    bad["todos"] = serde_json::json!([
        { "id": "t1", "title": "T1", "requirement_background": "bg", "instructions": "i",
          "depends_on": ["t2"], "acceptance": { "criteria": "c" } },
        { "id": "t2", "title": "T2", "requirement_background": "bg", "instructions": "i",
          "depends_on": ["t1"], "acceptance": { "criteria": "c" } },
    ]);
    bad
}

async fn create_demo(state: &Arc<opencoder_web::AppState>) {
    let (status, v) = call(
        app(state.clone()),
        "POST",
        "/api/todo/templates",
        Some(serde_json::json!({"name": "demo", "spec": spec("act")})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
}

/// T-1: create → list → get → meta → context roundtrip; duplicate 409.
#[tokio::test]
async fn template_crud_roundtrip() {
    let _g = share().await;
    let state = state().await;
    let a = || app(state.clone());
    let (status, v) = call(
        a(),
        "POST",
        "/api/todo/templates",
        Some(serde_json::json!({"name": "demo", "description": "d", "spec": spec("act")})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["template"]["current"], "v1");
    assert_eq!(v["template"]["versions"].as_array().unwrap().len(), 1);

    let (status, v) = call(
        a(),
        "POST",
        "/api/todo/templates",
        Some(serde_json::json!({"name": "demo", "spec": spec("act")})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");

    let (status, v) = call(a(), "GET", "/api/todo/templates", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["templates"].as_array().unwrap().len(), 1);
    assert_eq!(v["templates"][0]["name"], "demo");

    let (status, v) = call(a(), "GET", "/api/todo/templates/demo", None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["template"]["name"], "demo");
    assert_eq!(v["env_by_version"]["v1"], serde_json::Value::Null);

    let (status, v) = call(a(), "GET", "/api/todo/templates/demo/todo.json", None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["template"]["current"], "v1");

    let (status, v) = call(a(), "GET", "/api/todo/templates/demo/v1/context.json", None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["todos"][0]["agent"], "act");

    let (status, _) = call(a(), "GET", "/api/todo/templates/missing", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// T-2: spec validation gates create — cycles and unknown agents are 400s
/// and leave nothing on disk.
#[tokio::test]
async fn create_rejects_invalid_specs() {
    let _g = share().await;
    let state = state().await;
    let a = || app(state.clone());
    let (status, v) = call(
        a(),
        "POST",
        "/api/todo/templates",
        Some(serde_json::json!({"name": "bad", "spec": cycle_spec()})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("cycle"));

    let (status, v) = call(
        a(),
        "POST",
        "/api/todo/templates",
        Some(serde_json::json!({"name": "bad", "spec": spec("nope")})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("unknown agent"));

    let (status, _) = call(a(), "GET", "/api/todo/templates/bad", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// T-3: metadata merge-patch — description updates, `current` must name a
/// known version.
#[tokio::test]
async fn update_meta_patches_description_and_current() {
    let _g = share().await;
    let state = state().await;
    let a = || app(state.clone());
    create_demo(&state).await;

    let (status, v) = call(
        a(),
        "PUT",
        "/api/todo/templates/demo/todo.json",
        Some(serde_json::json!({"description": "patched", "current": "v9"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("unknown version v9"));

    let (status, v) = call(
        a(),
        "PUT",
        "/api/todo/templates/demo/todo.json",
        Some(serde_json::json!({"description": "patched"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["template"]["description"], "patched");
    assert_eq!(
        v["template"]["current"], "v1",
        "absent keys keep their value"
    );
}

/// T-4: context updates are validated; new-version forks; current is guarded.
#[tokio::test]
async fn context_update_and_version_lifecycle() {
    let _g = share().await;
    let state = state().await;
    let a = || app(state.clone());
    create_demo(&state).await;

    let mut cycle = cycle_spec();
    cycle["id"] = serde_json::json!("wf-2");
    let (status, v) = call(
        a(),
        "PUT",
        "/api/todo/templates/demo/v1/context.json",
        Some(cycle),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("spec 校验失败"));

    let mut updated = spec("act");
    updated["objective"] = serde_json::json!("v2 objective");
    let (status, v) = call(
        a(),
        "PUT",
        "/api/todo/templates/demo/v1/context.json",
        Some(updated),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");

    let (status, v) = call(
        a(),
        "POST",
        "/api/todo/templates/demo/new-version",
        Some(serde_json::json!({"note": "fork"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["version"], "v2");

    let (_, v) = call(a(), "GET", "/api/todo/templates/demo/todo.json", None).await;
    assert_eq!(v["template"]["current"], "v2");
    assert_eq!(v["template"]["versions"].as_array().unwrap().len(), 2);
    let (_, v) = call(a(), "GET", "/api/todo/templates/demo/v2/context.json", None).await;
    assert_eq!(v["objective"], "v2 objective", "context copied verbatim");

    let (status, v) = call(a(), "DELETE", "/api/todo/templates/demo/v2", None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    assert!(v["error"]
        .as_str()
        .unwrap()
        .contains("不能删除 current 版本 v2"));

    let (status, v) = call(a(), "DELETE", "/api/todo/templates/demo/v1", None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (status, _) = call(a(), "GET", "/api/todo/templates/demo/v1/context.json", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, v) = call(a(), "DELETE", "/api/todo/templates/demo", None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (status, _) = call(a(), "GET", "/api/todo/templates/demo", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// T-5: env binding requires an existing env, clears to null, rides along on
/// new-version forks.
#[tokio::test]
async fn env_binding_lifecycle() {
    let (_g, root) = share().await;
    let state = state().await;
    let a = || app(state.clone());
    create_demo(&state).await;

    let (status, v) = call(
        a(),
        "PUT",
        "/api/todo/templates/demo/v1/env.json",
        Some(serde_json::json!({"env": "nope"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("env 不存在: nope"));

    let env_dir = root.join("env").join("dev");
    std::fs::create_dir_all(&env_dir).unwrap();
    std::fs::write(
        env_dir.join("context.json"),
        serde_json::json!({"name": "dev", "tools": [], "env_vars": {}}).to_string(),
    )
    .unwrap();
    let (status, v) = call(
        a(),
        "PUT",
        "/api/todo/templates/demo/v1/env.json",
        Some(serde_json::json!({"env": "dev"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (status, v) = call(a(), "GET", "/api/todo/templates/demo/v1/env.json", None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["env"], "dev");
    let (_, v) = call(a(), "GET", "/api/todo/templates/demo", None).await;
    assert_eq!(v["env_by_version"]["v1"], "dev");
    // Fork carries the binding; clearing writes an explicit null tombstone.
    let (status, v) = call(
        a(),
        "POST",
        "/api/todo/templates/demo/new-version",
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (_, v) = call(a(), "GET", "/api/todo/templates/demo/v2/env.json", None).await;
    assert_eq!(v["env"], "dev", "binding copied to the fork");
    let (status, v) = call(
        a(),
        "PUT",
        "/api/todo/templates/demo/v2/env.json",
        Some(serde_json::json!({"env": null})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (_, v) = call(a(), "GET", "/api/todo/templates/demo/v2/env.json", None).await;
    assert_eq!(v["env"], serde_json::Value::Null);
}

/// T-6: traversal-shaped template names are rejected at body validation.
#[tokio::test]
async fn template_name_traversal_rejected() {
    let _g = share().await;
    let state = state().await;
    let a = || app(state.clone());
    for name in ["../x", "a/b", ".."] {
        let (status, v) = call(
            a(),
            "POST",
            "/api/todo/templates",
            Some(serde_json::json!({"name": name, "spec": spec("act")})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "name {name:?}: {v}");
    }
}
