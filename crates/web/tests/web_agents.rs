//! `/api/agents` REST contract tests. The agents root lives behind a
//! process-global override (`opencoder_core::agent::set_agents_dir_override`),
//! so every test holds ONE static lock for its whole body (mirrors the
//! `opencoder-agents` testutil). Thin router + oneshot (same shape as
//! `web_envs.rs`); reload fan-out is observed through a stolen drain-cmd
//! receiver, exactly like the envs activation test.

use std::sync::{Arc, Mutex, MutexGuard};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::{get, patch, put};
use axum::Router;
use tower::ServiceExt;

use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_store::{LibsqlStore, Store};

/// Serializes tests that touch the process-global agents-root override.
static OVERRIDE_LOCK: Mutex<()> = Mutex::new(());

/// Point the agents root at a fresh tempdir under the override lock; the
/// guard must be held across every agents call in the test body.
fn scoped() -> (tempfile::TempDir, MutexGuard<'static, ()>) {
    let dir = tempfile::tempdir().unwrap();
    let guard = OVERRIDE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    opencoder_core::agent::set_agents_dir_override(Some(dir.path().to_path_buf()));
    (dir, guard)
}

fn app(state: Arc<opencoder_web::AppState>) -> Router {
    Router::new()
        .route(
            "/api/agents",
            get(opencoder_web::api_agents::list).post(opencoder_web::api_agents::create),
        )
        .route(
            "/api/agents/active",
            patch(opencoder_web::api_agents::patch_active),
        )
        .route(
            "/api/agents/:name/meta",
            get(opencoder_web::api_agents::meta),
        )
        .route(
            "/api/agents/:name",
            put(opencoder_web::api_agents::update).delete(opencoder_web::api_agents::delete),
        )
        .with_state(state)
}

async fn state() -> Arc<opencoder_web::AppState> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let workdir = std::env::temp_dir().join(format!("oc-web-agents-{}", uuid::Uuid::new_v4()));
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

/// Register a live handle under `sid` and steal its drain-cmd receiver —
/// the fan-out seam (same as `web_envs.rs`).
async fn live_handle(
    state: &opencoder_web::AppState,
    sid: &str,
) -> tokio::sync::mpsc::UnboundedReceiver<opencoder_web::cmd::DrainCmd> {
    let handle = opencoder_web::handle::SessionHandle::new();
    let rx = handle.cmd_rx.lock().unwrap().take().expect("fresh handle");
    state.handles.lock().await.insert(sid.to_string(), handle);
    rx
}

/// Assert the next drained command is a ReloadConfig fan-out.
fn expect_reload(rx: &mut tokio::sync::mpsc::UnboundedReceiver<opencoder_web::cmd::DrainCmd>) {
    match rx.try_recv() {
        Ok(opencoder_web::cmd::DrainCmd::ReloadConfig) => {}
        other => panic!("expected ReloadConfig, got {other:?}"),
    }
}

/// Assert NO further fan-out arrived (silence).
fn expect_silent(rx: &mut tokio::sync::mpsc::UnboundedReceiver<opencoder_web::cmd::DrainCmd>) {
    assert!(rx.try_recv().is_err(), "unexpected ReloadConfig fan-out");
}

/// Write a live `prompts/<name>` pool (meta current=v1 + one version dir) —
/// the minimum `resource_current_version_dir` needs to resolve.
fn seed_prompt_pool(root: &std::path::Path, name: &str) {
    let dir = root.join("prompts").join(name);
    std::fs::create_dir_all(dir.join("v1")).unwrap();
    std::fs::write(
        dir.join("meta.json"),
        serde_json::json!({ "name": name, "current": 1, "history": [1] }).to_string(),
    )
    .unwrap();
    std::fs::write(dir.join("v1").join("soul.md"), "seeded prompt\n").unwrap();
}

#[tokio::test]
async fn empty_root_lists_null_active() {
    let state = state().await;
    let _scoped = scoped();
    let (status, v) = call(app(state.clone()), "GET", "/api/agents", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["ok"], true);
    assert_eq!(v["active"], serde_json::Value::Null);
    assert_eq!(v["agents"].as_array().map(Vec::len), Some(0));
}

#[tokio::test]
async fn cards_crud_activation_and_listing() {
    let state = state().await;
    let _scoped = scoped();
    seed_prompt_pool(_scoped.0.path(), "pack");
    // "b" carries a live prompt ref (activatable); "a" stays plain.
    let (status, v) = call(
        app(state.clone()),
        "POST",
        "/api/agents",
        Some(serde_json::json!({ "name": "b", "current": { "prompt": "pack" } })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let (status, v) = call(
        app(state.clone()),
        "POST",
        "/api/agents",
        Some(serde_json::json!({ "name": "a" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    // Duplicate ⇒ 409; reserved/illegal names ⇒ 400.
    let (status, v) = call(
        app(state.clone()),
        "POST",
        "/api/agents",
        Some(serde_json::json!({ "name": "a" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    for bad in ["active", "prompts", "../x", "  "] {
        let (status, v) = call(
            app(state.clone()),
            "POST",
            "/api/agents",
            Some(serde_json::json!({ "name": bad })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {v}");
    }

    // Activate ⇒ listing is sorted by name and carries the marker.
    let (status, v) = call(
        app(state.clone()),
        "PATCH",
        "/api/agents/active",
        Some(serde_json::json!({ "active": "b" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["active"], "b");

    let (status, v) = call(app(state.clone()), "GET", "/api/agents", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["active"], "b");
    let names: Vec<&str> = v["agents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["a", "b"]);
    for key in ["current", "references", "updated_at"] {
        assert!(v["agents"][0].get(key).is_some(), "lacks {key}");
    }

    // PUT rewrites refs (history entry per changed field); meta exposes it.
    let (status, v) = call(
        app(state.clone()),
        "PUT",
        "/api/agents/a",
        Some(serde_json::json!({ "current": { "prompt": "pack" } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (status, v) = call(app(state.clone()), "GET", "/api/agents/a/meta", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["meta"]["name"], "a");
    assert_eq!(v["meta"]["current"]["prompt"], "pack");
    let fields: Vec<&str> = v["meta"]["history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["field"].as_str().unwrap())
        .collect();
    assert_eq!(fields, vec!["prompt"]);

    // Missing card ⇒ 404 on meta / PUT / DELETE; unknown activation ⇒ 404.
    for (method, uri) in [
        ("GET", "/api/agents/ghost/meta"),
        ("PUT", "/api/agents/ghost"),
        ("DELETE", "/api/agents/ghost"),
    ] {
        let body = (method == "PUT").then(|| serde_json::json!({ "current": {} }));
        let (status, v) = call(app(state.clone()), method, uri, body).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}: {v}");
    }
    let (status, _) = call(
        app(state.clone()),
        "PATCH",
        "/api/agents/active",
        Some(serde_json::json!({ "active": "ghost" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn repeat_activation_fans_reload_once() {
    let state = state().await;
    let _scoped = scoped();
    seed_prompt_pool(_scoped.0.path(), "pack");
    call(
        app(state.clone()),
        "POST",
        "/api/agents",
        Some(serde_json::json!({ "name": "same", "current": { "prompt": "pack" } })),
    )
    .await;
    let mut cmd_rx = live_handle(&state, "s1").await;

    let (status, v) = call(
        app(state.clone()),
        "PATCH",
        "/api/agents/active",
        Some(serde_json::json!({ "active": "same" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    expect_reload(&mut cmd_rx);

    // Same value again ⇒ `unchanged`, no second fan-out.
    let (status, v) = call(
        app(state.clone()),
        "PATCH",
        "/api/agents/active",
        Some(serde_json::json!({ "active": "same" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["unchanged"], true);
    expect_silent(&mut cmd_rx);

    // Deactivate (`null`) IS a change ⇒ one more fan-out.
    let (status, v) = call(
        app(state.clone()),
        "PATCH",
        "/api/agents/active",
        Some(serde_json::json!({ "active": null })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["active"], serde_json::Value::Null);
    expect_reload(&mut cmd_rx);
}

#[tokio::test]
async fn blank_active_name_is_400_not_deactivation() {
    let state = state().await;
    let _scoped = scoped();
    seed_prompt_pool(_scoped.0.path(), "pack");
    call(
        app(state.clone()),
        "POST",
        "/api/agents",
        Some(serde_json::json!({ "name": "on", "current": { "prompt": "pack" } })),
    )
    .await;
    call(
        app(state.clone()),
        "PATCH",
        "/api/agents/active",
        Some(serde_json::json!({ "active": "on" })),
    )
    .await;
    let (status, v) = call(
        app(state.clone()),
        "PATCH",
        "/api/agents/active",
        Some(serde_json::json!({ "active": "   " })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert_eq!(
        opencoder_core::agent::active_agent().as_deref(),
        Some("on"),
        "blank must not deactivate"
    );
}

/// PUT fans ReloadConfig only for the ACTIVE card — a non-active card
/// cannot change any live session's chain.
#[tokio::test]
async fn put_fans_reload_only_for_active_card() {
    let state = state().await;
    let _scoped = scoped();
    seed_prompt_pool(_scoped.0.path(), "old-pack");
    seed_prompt_pool(_scoped.0.path(), "pack");
    // "hot" holds a live prompt ref (activatable); "cold" stays plain.
    let bodies = [
        ("hot", serde_json::json!({ "prompt": "old-pack" })),
        ("cold", serde_json::json!({})),
    ];
    for (name, current) in bodies {
        call(
            app(state.clone()),
            "POST",
            "/api/agents",
            Some(serde_json::json!({ "name": name, "current": current })),
        )
        .await;
    }
    call(
        app(state.clone()),
        "PATCH",
        "/api/agents/active",
        Some(serde_json::json!({ "active": "hot" })),
    )
    .await;
    let mut cmd_rx = live_handle(&state, "s1").await;

    let (status, v) = call(
        app(state.clone()),
        "PUT",
        "/api/agents/cold",
        Some(serde_json::json!({ "current": { "prompt": "pack" } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    expect_silent(&mut cmd_rx);

    let (status, v) = call(
        app(state.clone()),
        "PUT",
        "/api/agents/hot",
        Some(serde_json::json!({ "current": { "prompt": "pack" } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    expect_reload(&mut cmd_rx);
}

/// DELETE of the ACTIVE card clears the marker first and fans out; the
/// shared pools are never touched by card deletion.
#[tokio::test]
async fn delete_active_card_clears_marker_and_fans_reload() {
    let state = state().await;
    let _scoped = scoped();
    seed_prompt_pool(_scoped.0.path(), "pack");
    call(
        app(state.clone()),
        "POST",
        "/api/agents",
        Some(serde_json::json!({ "name": "gone", "current": { "prompt": "pack" } })),
    )
    .await;
    call(
        app(state.clone()),
        "PATCH",
        "/api/agents/active",
        Some(serde_json::json!({ "active": "gone" })),
    )
    .await;
    let mut cmd_rx = live_handle(&state, "s1").await;

    let (status, v) = call(app(state.clone()), "DELETE", "/api/agents/gone", None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(opencoder_core::agent::active_agent(), None);
    expect_reload(&mut cmd_rx);
    let (_, v) = call(app(state.clone()), "GET", "/api/agents", None).await;
    assert_eq!(v["active"], serde_json::Value::Null);
    assert_eq!(v["agents"].as_array().map(Vec::len), Some(0));
}

/// Activating a card whose prompt reference has no live version must fail
/// (preflight) AND roll the marker back — the previous active survives.
#[tokio::test]
async fn patch_preflight_missing_prompt_rolls_back() {
    let state = state().await;
    let _scoped = scoped();
    seed_prompt_pool(_scoped.0.path(), "pack");
    call(
        app(state.clone()),
        "POST",
        "/api/agents",
        Some(serde_json::json!({ "name": "plain", "current": { "prompt": "pack" } })),
    )
    .await;
    call(
        app(state.clone()),
        "POST",
        "/api/agents",
        Some(serde_json::json!({ "name": "broken", "current": { "prompt": "ghost-pack" } })),
    )
    .await;
    let (status, v) = call(
        app(state.clone()),
        "PATCH",
        "/api/agents/active",
        Some(serde_json::json!({ "active": "plain" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");

    let (status, v) = call(
        app(state.clone()),
        "PATCH",
        "/api/agents/active",
        Some(serde_json::json!({ "active": "broken" })),
    )
    .await;
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY,
        "preflight failure must be 4xx: {status} {v}"
    );
    assert_eq!(v["ok"], false);
    // Rollback: the marker still names the previous agent.
    let (_, v) = call(app(state.clone()), "GET", "/api/agents", None).await;
    assert_eq!(v["active"], "plain", "marker must roll back: {v}");
}

/// Activating a card with NO prompt reference must be rejected (it would
/// resolve to None and silently fall back to act) and roll the marker back.
#[tokio::test]
async fn patch_preflight_promptless_card_rejected_and_rolls_back() {
    let state = state().await;
    let _scoped = scoped();
    seed_prompt_pool(_scoped.0.path(), "pack");
    call(
        app(state.clone()),
        "POST",
        "/api/agents",
        Some(serde_json::json!({ "name": "plain", "current": { "prompt": "pack" } })),
    )
    .await;
    let (status, v) = call(
        app(state.clone()),
        "PATCH",
        "/api/agents/active",
        Some(serde_json::json!({ "active": "plain" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");

    // "empty" parses but has no prompt reference at all.
    call(
        app(state.clone()),
        "POST",
        "/api/agents",
        Some(serde_json::json!({ "name": "empty" })),
    )
    .await;
    let (status, v) = call(
        app(state.clone()),
        "PATCH",
        "/api/agents/active",
        Some(serde_json::json!({ "active": "empty" })),
    )
    .await;
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY,
        "promptless activation must be 4xx: {status} {v}"
    );
    assert_eq!(v["ok"], false);
    assert!(
        v["error"].as_str().unwrap().contains("prompt"),
        "error must mention the missing prompt: {v}"
    );
    // Rollback: the marker still names the previous agent.
    let (_, v) = call(app(state.clone()), "GET", "/api/agents", None).await;
    assert_eq!(v["active"], "plain", "marker must roll back: {v}");
}
