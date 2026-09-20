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
            "/api/todo/validate-files",
            post(opencoder_web::api_todo_directory::validate_files),
        )
        .route(
            "/api/todo/templates/:name/:version/files",
            get(opencoder_web::api_todo_directory::files),
        )
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
                config_home: None,
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
    let (status, _) = call(
        a(),
        "PUT",
        "/api/todo/templates/demo/v1/context.json",
        Some(spec("act")),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, v) = call(
        a(),
        "POST",
        "/api/todo/templates/demo/new-version",
        Some(
            serde_json::json!({"source_version":"v1","expected_current":"v1","spec":cycle_spec()}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["diagnostics"].is_array());
    let mut updated = spec("act");
    updated["objective"] = serde_json::json!("v2 objective");
    let body = serde_json::json!({"source_version":"v1","expected_current":"v1","spec":updated});
    let (status, v) = call(
        a(),
        "POST",
        "/api/todo/templates/demo/new-version",
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["version"], "v2");
    let (status, _) = call(
        a(),
        "POST",
        "/api/todo/templates/demo/new-version",
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (_, old) = call(a(), "GET", "/api/todo/templates/demo/v1/context.json", None).await;
    assert_ne!(old["objective"], "v2 objective");
    let (_, next) = call(a(), "GET", "/api/todo/templates/demo/v2/context.json", None).await;
    assert_eq!(next["objective"], "v2 objective");
    let (status, _) = call(a(), "DELETE", "/api/todo/templates/demo/v2", None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        call(a(), "DELETE", "/api/todo/templates/demo/v1", None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        call(a(), "GET", "/api/todo/templates/demo/v1/context.json", None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(a(), "DELETE", "/api/todo/templates/demo", None)
            .await
            .0,
        StatusCode::OK
    );
}

/// T-5: env binding requires an existing env, clears to null, rides along on
/// new-version forks.
#[tokio::test]
async fn env_binding_lifecycle() {
    let (_g, root) = share().await;
    let state = state().await;
    let a = || app(state.clone());
    create_demo(&state).await;
    let (status, v) = call(a(), "POST", "/api/todo/templates/demo/new-version", Some(serde_json::json!({"source_version":"v1","expected_current":"v1","binding":{"env":"nope"}}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert_eq!(v["diagnostics"][0]["path"], "env.json");
    let env_dir = root.join("env/dev");
    std::fs::create_dir_all(&env_dir).unwrap();
    std::fs::write(
        env_dir.join("context.json"),
        serde_json::json!({"name":"dev","tools":[],"env_vars":{}}).to_string(),
    )
    .unwrap();
    let (status, v) = call(a(), "POST", "/api/todo/templates/demo/new-version", Some(serde_json::json!({"source_version":"v1","expected_current":"v1","binding":{"env":"dev"}}))).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["version"], "v2");
    let (_, v) = call(a(), "GET", "/api/todo/templates/demo/v1/env.json", None).await;
    assert!(v["env"].is_null());
    let (_, v) = call(a(), "GET", "/api/todo/templates/demo/v2/env.json", None).await;
    assert_eq!(v["env"], "dev");
    let (_, v) = call(
        a(),
        "POST",
        "/api/todo/templates/demo/new-version",
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(v["version"], "v3");
    let (_, v) = call(a(), "GET", "/api/todo/templates/demo/v3/env.json", None).await;
    assert_eq!(v["env"], "dev");
    let (status, _) = call(a(), "POST", "/api/todo/templates/demo/new-version", Some(serde_json::json!({"source_version":"v3","expected_current":"v3","binding":{"env":null}}))).await;
    assert_eq!(status, StatusCode::OK);
    let (_, v) = call(a(), "GET", "/api/todo/templates/demo", None).await;
    assert_eq!(v["env_by_version"]["v2"], "dev");
    assert!(v["env_by_version"]["v4"].is_null());
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

#[tokio::test]
async fn directory_errors_preserve_raw_files_and_never_publish_invalid_or_stale_edits() {
    let (_guard, root) = share().await;
    let state = state().await;
    create_demo(&state).await;
    let a = || app(state.clone());
    let (_, bundle) = call(a(), "GET", "/api/todo/templates/demo/v1/files", None).await;
    let mut files = bundle["files"].clone();
    files["todos/t1/instructions.md"] = serde_json::json!("# Edited\n\n完整上下文\n");
    let mut invalid = files.clone();
    invalid["todos/t1/task.json"] = serde_json::json!("{\ninvalid");
    let body = |files| serde_json::json!({"source_version":"v1","expected_revision":bundle["revision"],"files":files});
    let (status, result) = call(
        a(),
        "POST",
        "/api/todo/templates/demo/new-version",
        Some(body(invalid)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(result["diagnostics"][0]["path"], "todos/t1/task.json");
    assert!(!root.join("todo/demo/v2").exists());
    let (status, _) = call(
        a(),
        "POST",
        "/api/todo/templates/demo/new-version",
        Some(body(files.clone())),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        call(
            a(),
            "POST",
            "/api/todo/templates/demo/new-version",
            Some(body(files))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert!(!root.join("todo/demo/v3").exists());
    let (_, original) = call(a(), "GET", "/api/todo/templates/demo/v1/files", None).await;
    assert_eq!(original["files"], bundle["files"]);
    std::fs::write(
        root.join("todo/demo/v2/env.json"),
        r#"{"env":"missing-env"}"#,
    )
    .unwrap();
    let (status, broken) = call(a(), "GET", "/api/todo/templates/demo/v2/files", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(broken["files"]["env.json"], r#"{"env":"missing-env"}"#);
    assert_eq!(broken["diagnostics"][0]["path"], "env.json");
    assert_eq!(
        call(
            a(),
            "POST",
            "/api/todo/validate-files",
            Some(serde_json::json!({"files":broken["files"]}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (_, tree) = call(
        a(),
        "GET",
        "/api/todo/templates/demo/v1/files?tree=true",
        None,
    )
    .await;
    assert!(tree.get("files").is_none());
    assert_eq!(tree["entries"].as_array().unwrap().len(), 7);
}

/// Fork helper: low-effort body (note only) is accepted without expectation
/// guards and flips `current` to the fresh version.
async fn fork(state: &Arc<opencoder_web::AppState>, note: &str) -> serde_json::Value {
    let (status, v) = call(
        app(state.clone()),
        "POST",
        "/api/todo/templates/demo/new-version",
        Some(serde_json::json!({"note": note})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    v
}

fn version_names(meta: &serde_json::Value) -> Vec<&str> {
    meta["template"]["versions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["version"].as_str().unwrap())
        .collect()
}

/// Retention keeps only the most recent 10 versions: forking past the cap
/// prunes the oldest directories, `todo.json` stops advertising them and
/// reads of a pruned version 404.
#[tokio::test]
async fn retention_keeps_recent_ten_versions_and_prunes_oldest() {
    let (_g, root) = share().await;
    let state = state().await;
    create_demo(&state).await;
    for i in 2..=10 {
        fork(&state, &format!("v{i}")).await;
    }

    let v11 = fork(&state, "v11").await;
    assert_eq!(v11["pruned"], serde_json::json!(["v1"]), "{v11}");
    let v12 = fork(&state, "v12").await;
    assert_eq!(v12["pruned"], serde_json::json!(["v2"]), "{v12}");

    let (_, meta) = call(app(state.clone()), "GET", "/api/todo/templates/demo", None).await;
    assert_eq!(
        version_names(&meta),
        vec!["v3", "v4", "v5", "v6", "v7", "v8", "v9", "v10", "v11", "v12"]
    );
    assert_eq!(meta["template"]["current"], "v12");

    let mut on_disk: Vec<String> = std::fs::read_dir(root.join("todo/demo"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| root.join("todo/demo").join(n).is_dir())
        .collect();
    on_disk.sort();
    assert_eq!(
        on_disk,
        vec!["v10", "v11", "v12", "v3", "v4", "v5", "v6", "v7", "v8", "v9"]
    );

    let (status, v) = call(
        app(state.clone()),
        "GET",
        "/api/todo/templates/demo/v1/files",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{v}");
}

/// Pinning `current` back to an old version never prunes by itself: only the
/// next fork grows the list. The fork flips `current` to the fresh version,
/// so the retention cut follows "most recent 10" — the formerly pinned
/// version loses its slot the moment it is no longer current.
#[tokio::test]
async fn retention_pinned_current_is_untouched_until_next_fork() {
    let (_g, root) = share().await;
    let state = state().await;
    create_demo(&state).await;
    for i in 2..=10 {
        fork(&state, &format!("v{i}")).await;
    }

    let (status, meta) = call(
        app(state.clone()),
        "PUT",
        "/api/todo/templates/demo/todo.json",
        Some(serde_json::json!({"current": "v1"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{meta}");
    assert_eq!(meta["template"]["current"], "v1");
    assert_eq!(
        meta["template"]["versions"].as_array().unwrap().len(),
        10,
        "flipping current never prunes"
    );
    for i in 1..=10 {
        assert!(root.join(format!("todo/demo/v{i}")).is_dir());
    }

    let v11 = fork(&state, "v11").await;
    assert_eq!(v11["pruned"], serde_json::json!(["v1"]), "{v11}");
    assert_eq!(v11["template"]["current"], "v11");
    assert!(!root.join("todo/demo/v1").exists());
}
