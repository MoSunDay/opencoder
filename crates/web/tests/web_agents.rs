//! `/api/agents` REST contract tests. The agents root lives behind a
//! process-global override (`opencoder_core::agent::set_agents_dir_override`),
//! so every test holds ONE static lock for its whole body (mirrors the
//! `opencoder-agents` testutil). Thin router + oneshot (same shape as
//! `web_envs.rs`); reload fan-out is observed through a stolen drain-cmd
//! receiver, exactly like the envs tests. 全局激活端点已移除，卡片写一律
//! 无条件扇出 ReloadConfig。

use std::sync::{Arc, Mutex, MutexGuard};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::{get, put};
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
// (`expect_silent` removed with the activation gate: every card write now
// fans out unconditionally, so silence is never expected.)

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
async fn empty_root_lists_cards_only_without_active_field() {
    let state = state().await;
    let _scoped = scoped();
    let (status, v) = call(app(state.clone()), "GET", "/api/agents", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["ok"], true);
    // 全局激活已移除：list 响应不再携带 `active` 字段。
    assert!(v.get("active").is_none(), "list must not carry `active`: {v}");
    // Registry-only: no builtin scheduling roles leak into the list.
    let agents = v["agents"].as_array().unwrap();
    assert!(
        agents.is_empty(),
        "empty agents root must list no cards: {agents:?}"
    );
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
    for bad in ["prompts", "../x", "  "] {
        let (status, v) = call(
            app(state.clone()),
            "POST",
            "/api/agents",
            Some(serde_json::json!({ "name": bad })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {v}");
    }

    // Listing is sorted by name; the global activation pointer is gone.
    let (status, v) = call(app(state.clone()), "GET", "/api/agents", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(v.get("active").is_none(), "list must not carry `active`: {v}");
    // Registry-only: created cards are the whole list, no builtin roles.
    let names: Vec<&str> = v["agents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["a", "b"]);
    assert!(v["agents"]
        .as_array()
        .unwrap()
        .iter()
        .all(|a| a["builtin"] == false));
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

    // Missing card ⇒ 404 on meta / PUT / DELETE.
    for (method, uri) in [
        ("GET", "/api/agents/ghost/meta"),
        ("PUT", "/api/agents/ghost"),
        ("DELETE", "/api/agents/ghost"),
    ] {
        let body = (method == "PUT").then(|| serde_json::json!({ "current": {} }));
        let (status, v) = call(app(state.clone()), method, uri, body).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}: {v}");
    }
}

/// Pickers (SPA `@`/`/agent`, TUI `/agent`) render the one-line identity:
/// list items carry `description` taken from the card's prompt-pool
/// `soul.md` FIRST non-empty line (leading blank lines skipped); a card
/// without a resolvable prompt reference falls back to the generic
/// `Custom agent <name>` label — the same fallback `resolve_file_agent`
/// uses.
#[tokio::test]
async fn list_items_carry_soul_first_line_description_with_generic_fallback() {
    let state = state().await;
    let _scoped = scoped();
    let root = _scoped.0.path();
    // Multi-line soul: the description skips leading blank/whitespace lines.
    let pool = root.join("prompts/writer");
    std::fs::create_dir_all(pool.join("v1")).unwrap();
    std::fs::write(
        pool.join("meta.json"),
        r#"{ "name": "writer", "current": 1, "history": [1] }"#,
    )
    .unwrap();
    std::fs::write(
        pool.join("v1").join("soul.md"),
        "\n  \nWriter soul: small diffs.\n",
    )
    .unwrap();
    let (status, v) = call(
        app(state.clone()),
        "POST",
        "/api/agents",
        Some(serde_json::json!({ "name": "writer", "current": { "prompt": "writer" } })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    // Plain card without a prompt reference: generic fallback label.
    let (status, v) = call(
        app(state.clone()),
        "POST",
        "/api/agents",
        Some(serde_json::json!({ "name": "plain" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");

    let (status, v) = call(app(state.clone()), "GET", "/api/agents", None).await;
    assert_eq!(status, StatusCode::OK);
    let agents = v["agents"].as_array().unwrap();
    let by_name = |n: &str| {
        agents
            .iter()
            .find(|a| a["name"] == n)
            .unwrap_or_else(|| panic!("missing card {n}: {agents:?}"))
            .clone()
    };
    assert_eq!(by_name("writer")["description"], "Writer soul: small diffs.");
    assert_eq!(by_name("plain")["description"], "Custom agent plain");
}

/// 全局激活端点已移除：PATCH /api/agents/active 落到 /api/agents/:name 的
/// put/delete 路由上，PATCH 方法不被允许 ⇒ 405。
#[tokio::test]
async fn patch_active_endpoint_is_gone() {
    let state = state().await;
    let _scoped = scoped();
    let (status, v) = call(
        app(state.clone()),
        "PATCH",
        "/api/agents/active",
        Some(serde_json::json!({ "active": "a" })),
    )
    .await;
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED,
        "activation endpoint must be gone: {status} {v}"
    );
}

/// 卡片写入无条件 fan-out ReloadConfig：激活判断已移除，任何成功的
/// PUT 都刷新活跃会话的池快照（重复写也各扇出一次）。
#[tokio::test]
async fn put_fans_reload_on_every_write() {
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
        "PUT",
        "/api/agents/same",
        Some(serde_json::json!({ "current": { "prompt": "pack" } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    expect_reload(&mut cmd_rx);

    // 相同内容再写一次 ⇒ 仍然无条件扇出。
    let (status, v) = call(
        app(state.clone()),
        "PUT",
        "/api/agents/same",
        Some(serde_json::json!({ "current": { "prompt": "pack" } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    expect_reload(&mut cmd_rx);
}

/// DELETE fans ReloadConfig unconditionally（无激活 marker 可清）；共享
/// 资源池不会被卡片删除触碰。
#[tokio::test]
async fn delete_card_fans_reload_without_marker() {
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
    let mut cmd_rx = live_handle(&state, "s1").await;

    let (status, v) = call(app(state.clone()), "DELETE", "/api/agents/gone", None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    expect_reload(&mut cmd_rx);
    let (_, v) = call(app(state.clone()), "GET", "/api/agents", None).await;
    assert!(v.get("active").is_none(), "list must not carry `active`: {v}");
    // Registry-only: deleting the last card empties the list, no builtin roles.
    let agents = v["agents"].as_array().unwrap();
    assert!(
        agents.is_empty(),
        "list must contain no builtin roles: {agents:?}"
    );
}

#[tokio::test]
async fn harness_settings_apply_to_builtin_and_custom_agents() {
    let state = state().await;
    let _scoped = scoped();
    let (status, body) = call(
        app(state.clone()),
        "PUT",
        "/api/agents/act",
        Some(serde_json::json!({"harness":"codex"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, body) = call(app(state.clone()), "GET", "/api/agents/act/meta", None).await;
    assert_eq!(body["meta"]["harness"], "codex");
    assert_eq!(
        opencoder_core::resolve_agent("act").unwrap().kind,
        opencoder_core::AgentKind::Act
    );
    let (status, _) = call(app(state.clone()), "DELETE", "/api/agents/act", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(
        app(state.clone()),
        "PUT",
        "/api/agents/act",
        Some(serde_json::json!({"harness":"unknown"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = call(
        app(state.clone()),
        "POST",
        "/api/agents",
        Some(serde_json::json!({"name":"wrapped","harness":"codex"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (_, body) = call(app(state.clone()), "GET", "/api/agents/wrapped/meta", None).await;
    assert_eq!(body["meta"]["harness"], "codex");
}
