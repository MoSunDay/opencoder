//! `/api/envs` REST contract tests. Each test isolates config discovery with
//! `scoped_config_home` so env dirs land in a temp home, never the real
//! `~/.opencoder`. Thin router + oneshot (same shape as `web_api_ops.rs`).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::{delete, get, post};
use axum::Router;
use tower::ServiceExt;

use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_store::{LibsqlStore, Store};

fn app(state: Arc<opencoder_web::AppState>) -> Router {
    Router::new()
        .route(
            "/api/envs",
            get(opencoder_web::api_envs::list)
                .post(opencoder_web::api_envs::create)
                .patch(opencoder_web::api_envs::patch),
        )
        .route(
            "/api/envs/:name/recapture",
            post(opencoder_web::api_envs::recapture),
        )
        .route("/api/envs/:name", delete(opencoder_web::api_envs::delete))
        .with_state(state)
}

async fn state() -> Arc<opencoder_web::AppState> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let workdir = std::env::temp_dir().join(format!("oc-web-envs-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workdir).ok();
    Arc::new(opencoder_web::AppState {
        client_override: Some(Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>),
        store,
        workdir,
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
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
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let v = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, v)
}

/// Seed a base-layer config (project `opencoder.json`) so captures have meat.
fn seed_base(workdir: &std::path::Path, model: &str) {
    std::fs::create_dir_all(workdir).unwrap();
    std::fs::write(
        workdir.join("opencoder.json"),
        serde_json::json!({ "model": model }).to_string(),
    )
    .unwrap();
}

fn env_config_model(home: &std::path::Path, name: &str) -> String {
    let p = home.join("envs").join(name).join("config.json");
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    v["model"].as_str().unwrap_or("").to_string()
}

#[tokio::test]
async fn list_reports_envs_and_active() {
    let state = state().await;
    let _iso = opencoder_core::scoped_config_home(state.workdir.clone());
    let (status, v) = call(app(state.clone()), "GET", "/api/envs", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["ok"], true);
    assert_eq!(v["active"], serde_json::Value::Null);
    assert_eq!(v["envs"].as_array().map(Vec::len), Some(0));

    // Create one env, then list again: name + path present, still inactive.
    let (status, v) = call(
        app(state.clone()),
        "POST",
        "/api/envs",
        Some(serde_json::json!({"name": "work"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (status, v) = call(app(state.clone()), "GET", "/api/envs", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["envs"][0]["name"], "work");
    assert!(v["envs"][0]["path"].as_str().unwrap().contains("envs"));
}

#[tokio::test]
async fn create_rejects_duplicate_and_bad_name() {
    let state = state().await;
    let _iso = opencoder_core::scoped_config_home(state.workdir.clone());
    let (status, _) = call(
        app(state.clone()),
        "POST",
        "/api/envs",
        Some(serde_json::json!({"name": "dup"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, v) = call(
        app(state.clone()),
        "POST",
        "/api/envs",
        Some(serde_json::json!({"name": "dup"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["ok"], false);

    // Path-ish and whitespace-only names must not pass.
    for bad in ["../evil", "  "] {
        let (status, v) = call(
            app(state.clone()),
            "POST",
            "/api/envs",
            Some(serde_json::json!({"name": bad})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {v}");
    }
}

#[tokio::test]
async fn create_with_capture_seeds_env_from_base_chain() {
    let state = state().await;
    let _iso = opencoder_core::scoped_config_home(state.workdir.clone());
    seed_base(&state.workdir, "prov/mo");
    let (status, v) = call(
        app(state.clone()),
        "POST",
        "/api/envs",
        Some(serde_json::json!({"name": "snap", "capture_current": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    // The captured env carries the project-layer model (WYSIWYG capture).
    assert_eq!(
        env_config_model(&state.workdir.join(".opencoder"), "snap"),
        "prov/mo"
    );
}

#[tokio::test]
async fn patch_activates_and_deactivates() {
    let state = state().await;
    let _iso = opencoder_core::scoped_config_home(state.workdir.clone());
    call(
        app(state.clone()),
        "POST",
        "/api/envs",
        Some(serde_json::json!({"name": "on"})),
    )
    .await;
    // Unknown env → 404, marker untouched.
    let (status, v) = call(
        app(state.clone()),
        "PATCH",
        "/api/envs",
        Some(serde_json::json!({"active": "ghost"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{v}");

    let (status, v) = call(
        app(state.clone()),
        "PATCH",
        "/api/envs",
        Some(serde_json::json!({"active": "on"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["active"], "on");
    assert_eq!(
        opencoder_core::config::envs::active_env().as_deref(),
        Some("on")
    );

    let (_, v) = call(app(state.clone()), "GET", "/api/envs", None).await;
    assert_eq!(v["active"], "on");

    // Deactivate with explicit null.
    let (status, v) = call(
        app(state.clone()),
        "PATCH",
        "/api/envs",
        Some(serde_json::json!({"active": null})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(opencoder_core::config::envs::active_env(), None);
}

#[tokio::test]
async fn recapture_updates_env_files_from_current_base() {
    let state = state().await;
    let _iso = opencoder_core::scoped_config_home(state.workdir.clone());
    seed_base(&state.workdir, "old/mo");
    call(
        app(state.clone()),
        "POST",
        "/api/envs",
        Some(serde_json::json!({"name": "rc", "capture_current": true})),
    )
    .await;
    // Base drifts; recapture pulls the new value into the env snapshot.
    seed_base(&state.workdir, "new/mo");
    let (status, v) = call(app(state.clone()), "POST", "/api/envs/rc/recapture", None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(
        env_config_model(&state.workdir.join(".opencoder"), "rc"),
        "new/mo"
    );

    let (status, v) = call(
        app(state.clone()),
        "POST",
        "/api/envs/ghost/recapture",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{v}");
}

#[tokio::test]
async fn delete_removes_env_and_clears_active_marker() {
    let state = state().await;
    let _iso = opencoder_core::scoped_config_home(state.workdir.clone());
    call(
        app(state.clone()),
        "POST",
        "/api/envs",
        Some(serde_json::json!({"name": "gone"})),
    )
    .await;
    call(
        app(state.clone()),
        "PATCH",
        "/api/envs",
        Some(serde_json::json!({"active": "gone"})),
    )
    .await;

    let (status, v) = call(app(state.clone()), "DELETE", "/api/envs/ghost", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{v}");

    let (status, v) = call(app(state.clone()), "DELETE", "/api/envs/gone", None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert!(!state
        .workdir
        .join(".opencoder")
        .join("envs")
        .join("gone")
        .exists());
    assert_eq!(opencoder_core::config::envs::active_env(), None);
    let (_, v) = call(app(state.clone()), "GET", "/api/envs", None).await;
    assert_eq!(v["envs"].as_array().map(Vec::len), Some(0));
}

#[tokio::test]
async fn activation_fans_reload_config_to_live_handles() {
    let state = state().await;
    let _iso = opencoder_core::scoped_config_home(state.workdir.clone());
    call(
        app(state.clone()),
        "POST",
        "/api/envs",
        Some(serde_json::json!({"name": "live"})),
    )
    .await;

    // Register a live handle and steal its drain-cmd receiver so we can
    // observe exactly what fan-out delivers.
    let handle = opencoder_web::handle::SessionHandle::new();
    let mut cmd_rx = handle
        .cmd_rx
        .lock()
        .unwrap()
        .take()
        .expect("fresh handle carries a receiver");
    state.handles.lock().await.insert("s1".to_string(), handle);

    let (status, v) = call(
        app(state.clone()),
        "PATCH",
        "/api/envs",
        Some(serde_json::json!({"active": "live"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    match cmd_rx.try_recv() {
        Ok(opencoder_web::cmd::DrainCmd::ReloadConfig) => {}
        other => panic!("expected ReloadConfig, got {other:?}"),
    }
}

/// P2: a blank `active` name must be a 400, not a silent deactivation —
/// only explicit `null` deactivates.
#[tokio::test]
async fn patch_rejects_blank_active_name_with_400() {
    let state = state().await;
    let _iso = opencoder_core::scoped_config_home(state.workdir.clone());
    call(
        app(state.clone()),
        "POST",
        "/api/envs",
        Some(serde_json::json!({"name": "blank"})),
    )
    .await;
    let (status, v) = call(
        app(state.clone()),
        "PATCH",
        "/api/envs",
        Some(serde_json::json!({"active": "   "})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert_eq!(
        opencoder_core::config::envs::active_env(),
        None,
        "blank name must not deactivate the active env"
    );
}

/// E-2: activating an env whose config.json is corrupt must be rejected (500)
/// with the marker rolled back — otherwise the next process start fails hard
/// while activation itself reported ok.
#[tokio::test]
async fn patch_rejects_env_with_unresolvable_config() {
    let state = state().await;
    let _iso = opencoder_core::scoped_config_home(state.workdir.clone());
    call(
        app(state.clone()),
        "POST",
        "/api/envs",
        Some(serde_json::json!({"name": "broken"})),
    )
    .await;
    std::fs::write(
        state
            .workdir
            .join(".opencoder")
            .join("envs")
            .join("broken")
            .join("config.json"),
        "{ not json",
    )
    .unwrap();

    let (status, v) = call(
        app(state.clone()),
        "PATCH",
        "/api/envs",
        Some(serde_json::json!({"active": "broken"})),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{v}");
    assert!(
        v["error"].as_str().unwrap_or("").contains("unresolvable"),
        "the resolution error must surface: {v}"
    );
    assert_eq!(
        opencoder_core::config::envs::active_env(),
        None,
        "failed preflight must restore the previous marker state"
    );
}

/// P2: re-activating the already-active env short-circuits — ok response,
/// no marker rewrite, no ReloadConfig fan-out.
#[tokio::test]
async fn patch_repeated_activation_short_circuits() {
    let state = state().await;
    let _iso = opencoder_core::scoped_config_home(state.workdir.clone());
    call(
        app(state.clone()),
        "POST",
        "/api/envs",
        Some(serde_json::json!({"name": "same"})),
    )
    .await;

    let handle = opencoder_web::handle::SessionHandle::new();
    let mut cmd_rx = handle
        .cmd_rx
        .lock()
        .unwrap()
        .take()
        .expect("fresh handle carries a receiver");
    state.handles.lock().await.insert("s1".to_string(), handle);

    let (status, _) = call(
        app(state.clone()),
        "PATCH",
        "/api/envs",
        Some(serde_json::json!({"active": "same"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let _ = cmd_rx.try_recv(); // drain the first (real) activation fan-out

    let (status, v) = call(
        app(state.clone()),
        "PATCH",
        "/api/envs",
        Some(serde_json::json!({"active": "same"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["unchanged"], true, "repeat activation reports unchanged");
    assert!(
        cmd_rx.try_recv().is_err(),
        "re-activation must not fan ReloadConfig out again"
    );
}

/// E-3: concurrent activation PATCHes serialize on the in-process gate —
/// every response is 200 and the marker file stays parseable (never torn).
#[tokio::test]
async fn concurrent_activation_keeps_marker_intact() {
    let state = state().await;
    let _iso = opencoder_core::scoped_config_home(state.workdir.clone());
    for name in ["a", "b", "c"] {
        call(
            app(state.clone()),
            "POST",
            "/api/envs",
            Some(serde_json::json!({"name": name})),
        )
        .await;
    }

    let mut tasks = Vec::new();
    for name in ["a", "b", "c", "a", "b", "c"] {
        let state = state.clone();
        tasks.push(tokio::spawn(async move {
            let (status, v) = call(
                app(state),
                "PATCH",
                "/api/envs",
                Some(serde_json::json!({ "active": name })),
            )
            .await;
            (status, v)
        }));
    }
    for task in tasks {
        let (status, v) = task.await.unwrap();
        assert_eq!(status, StatusCode::OK, "{v}");
    }
    let marker = state.workdir.join(".opencoder").join("envs").join("active");
    let raw = std::fs::read_to_string(&marker).unwrap();
    let name = raw.trim();
    assert!(
        ["a", "b", "c"].contains(&name),
        "marker must hold exactly one valid env name, got {raw:?}"
    );
}
