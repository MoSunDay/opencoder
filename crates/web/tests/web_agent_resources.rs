//! `/api/agents/resources/*` REST contract tests — the shared versioned
//! pools (`prompts|skills|tools|memory`). Same isolation contract as
//! `web_agents.rs`: the agents root is a process-global override, so every
//! test holds one static lock for its whole body. Thin router + oneshot.

use std::sync::{Arc, Mutex, MutexGuard};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::{get, post, put};
use axum::Router;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use tower::ServiceExt;

use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_store::{LibsqlStore, Store};

static OVERRIDE_LOCK: Mutex<()> = Mutex::new(());

fn scoped() -> (tempfile::TempDir, MutexGuard<'static, ()>) {
    let dir = tempfile::tempdir().unwrap();
    let guard = OVERRIDE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    opencoder_core::agent::set_agents_dir_override(Some(dir.path().to_path_buf()));
    (dir, guard)
}

fn app(state: Arc<opencoder_web::AppState>) -> Router {
    Router::new()
        .route("/api/agents", post(opencoder_web::api_agents::create))
        .route(
            "/api/agents/active",
            axum::routing::patch(opencoder_web::api_agents::patch_active),
        )
        .route(
            "/api/agents/:name",
            put(opencoder_web::api_agents::update).delete(opencoder_web::api_agents::delete),
        )
        .route(
            "/api/agents/resources/:cat",
            get(opencoder_web::api_agent_resources::list)
                .post(opencoder_web::api_agent_resources::create),
        )
        .route(
            "/api/agents/resources/:cat/:name",
            put(opencoder_web::api_agent_resources::put_version)
                .delete(opencoder_web::api_agent_resources::delete),
        )
        .route(
            "/api/agents/resources/:cat/:name/meta",
            get(opencoder_web::api_agent_resources::meta),
        )
        .route(
            "/api/agents/resources/:cat/:name/rollback",
            post(opencoder_web::api_agent_resources::rollback),
        )
        .route(
            "/api/agents/resources/:cat/:name/versions/:v/files/*path",
            get(opencoder_web::api_agent_resources::read_file),
        )
        .layer(axum::extract::DefaultBodyLimit::max(6 * 1024 * 1024))
        .with_state(state)
}

async fn state() -> Arc<opencoder_web::AppState> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let workdir = std::env::temp_dir().join(format!("oc-web-agres-{}", uuid::Uuid::new_v4()));
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

fn save_body(name: &str, path: &str, content: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "files": [{ "path": path, "content_b64": B64.encode(content) }],
    })
}

/// Register a live handle + steal its drain-cmd receiver (fan-out seam).
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

#[tokio::test]
async fn version_lifecycle_and_file_roundtrip() {
    let state = state().await;
    let _scoped = scoped();
    let (_, v) = call(
        app(state.clone()),
        "GET",
        "/api/agents/resources/prompts",
        None,
    )
    .await;
    assert_eq!(v["ok"], true);
    assert_eq!(v["category"], "prompts");
    assert_eq!(v["resources"].as_array().map(Vec::len), Some(0));

    let (status, v) = call(
        app(state.clone()),
        "POST",
        "/api/agents/resources/prompts",
        save_body("pack", "soul.md", "hello"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["version"], 1);
    // Duplicate POST ⇒ 409 (PUT is the bump path).
    let (status, _) = call(
        app(state.clone()),
        "POST",
        "/api/agents/resources/prompts",
        save_body("pack", "soul.md", "hello"),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, v) = call(
        app(state.clone()),
        "PUT",
        "/api/agents/resources/prompts/pack",
        save_body("pack", "soul.md", "world"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["version"], 2);

    let (_, v) = call(
        app(state.clone()),
        "GET",
        "/api/agents/resources/prompts/pack/meta",
        None,
    )
    .await;
    assert_eq!(v["meta"]["current"], 2);
    assert_eq!(v["meta"]["history"], serde_json::json!([1, 2]));

    // Pinned-version file read round-trips base64.
    let (status, v) = call(
        app(state.clone()),
        "GET",
        "/api/agents/resources/prompts/pack/versions/1/files/soul.md",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["size"], 5);
    assert_eq!(
        B64.decode(v["content_b64"].as_str().unwrap()).unwrap(),
        b"hello"
    );
    let (_, v) = call(
        app(state.clone()),
        "GET",
        "/api/agents/resources/prompts/pack/versions/2/files/soul.md",
        None,
    )
    .await;
    assert_eq!(
        B64.decode(v["content_b64"].as_str().unwrap()).unwrap(),
        b"world"
    );

    // Rollback is a pointer switch: current 2 → 1, history intact.
    let (status, v) = call(
        app(state.clone()),
        "POST",
        "/api/agents/resources/prompts/pack/rollback",
        Some(serde_json::json!({ "version": 1 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["current"], 1);
    let (_, v) = call(
        app(state.clone()),
        "GET",
        "/api/agents/resources/prompts/pack/meta",
        None,
    )
    .await;
    assert_eq!(v["meta"]["current"], 1);
    assert_eq!(v["meta"]["history"], serde_json::json!([1, 2]));

    let (_, v) = call(
        app(state.clone()),
        "GET",
        "/api/agents/resources/prompts",
        None,
    )
    .await;
    assert_eq!(v["resources"][0]["name"], "pack");
    assert_eq!(v["resources"][0]["current"], 1);
    assert_eq!(v["resources"][0]["versions"], serde_json::json!([1, 2]));
}

#[tokio::test]
async fn rejects_bad_category_paths_shape_and_oversize() {
    let state = state().await;
    let _scoped = scoped();
    // Unknown category ⇒ 400 everywhere it is a path param.
    for (method, uri) in [
        ("GET", "/api/agents/resources/nope"),
        ("POST", "/api/agents/resources/nope"),
        ("GET", "/api/agents/resources/nope/x/meta"),
        ("PUT", "/api/agents/resources/nope/x"),
    ] {
        let body = method.ne("GET").then(|| save_body("x", "soul.md", "s"));
        let (status, v) = call(app(state.clone()), method, uri, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{method} {uri}: {v}");
    }

    // Traversal / absolute / empty paths ⇒ 400 (checked before any fs work).
    for path in ["../escape", "a/../../b", "/abs.md", "a//b.md", ""] {
        let (status, v) = call(
            app(state.clone()),
            "POST",
            "/api/agents/resources/tools",
            save_body("kit", path, "x"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}: {v}");
    }

    // Category shapes: prompts only the three sections, memory only
    // memory.md, skills SKILL.md-bearing.
    let cases = [
        ("prompts", "evil.md", true),
        ("prompts", "nested/soul.md", true),
        ("memory", "other.md", true),
        ("skills", "alpha/doc.md", true),
        ("skills", "beta/SKILL.md", false),
        ("skills", "gamma.md", false),
        ("tools", "deep/nested/run.sh", false),
    ];
    for (cat, path, want_bad) in cases {
        let (status, v) = call(
            app(state.clone()),
            "POST",
            &format!("/api/agents/resources/{cat}"),
            save_body("s", path, "x"),
        )
        .await;
        assert_eq!(
            status == StatusCode::BAD_REQUEST,
            want_bad,
            "{cat}/{path}: {v}"
        );
    }

    // Bad base64 ⇒ 400.
    let (status, _) = call(
        app(state.clone()),
        "POST",
        "/api/agents/resources/tools",
        serde_json::json!({ "name": "kit", "files": [{ "path": "run.sh", "content_b64": "@@@" }] }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Decoded total ≤ 1.5 MiB per request: exact cap passes, +1 byte fails.
    let cap = 1536 * 1024;
    let ok_body = serde_json::json!({
        "name": "big",
        "files": [{ "path": "run.sh", "content_b64": B64.encode(vec![b'a'; cap]) }],
    });
    let (status, v) = call(
        app(state.clone()),
        "POST",
        "/api/agents/resources/tools",
        ok_body,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let over = serde_json::json!({
        "name": "big2",
        "files": [
            { "path": "a.sh", "content_b64": B64.encode(vec![b'a'; cap / 2]) },
            { "path": "b.sh", "content_b64": B64.encode(vec![b'b'; cap / 2 + 1]) },
        ],
    });
    let (status, v) = call(
        app(state.clone()),
        "POST",
        "/api/agents/resources/tools",
        over,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "split payload must sum: {v}"
    );
}

/// ReloadConfig fans out only for writes touching the ACTIVE card's chain.
#[tokio::test]
async fn reload_only_for_active_chain_writes() {
    let state = state().await;
    let _scoped = scoped();
    call(
        app(state.clone()),
        "POST",
        "/api/agents/resources/prompts",
        save_body("pack", "soul.md", "one"),
    )
    .await;
    call(
        app(state.clone()),
        "POST",
        "/api/agents",
        serde_json::json!({ "name": "work", "current": { "prompt": "pack" } }),
    )
    .await;
    let mut cmd_rx = live_handle(&state, "s1").await;
    call(
        app(state.clone()),
        "PATCH",
        "/api/agents/active",
        serde_json::json!({ "active": "work" }),
    )
    .await;
    expect_reload(&mut cmd_rx); // the activation itself fans out once

    // Referenced resource PUT ⇒ fan-out.
    call(
        app(state.clone()),
        "PUT",
        "/api/agents/resources/prompts/pack",
        save_body("pack", "soul.md", "two"),
    )
    .await;
    expect_reload(&mut cmd_rx);

    // Unreferenced resource POST ⇒ silent disk write.
    call(
        app(state.clone()),
        "POST",
        "/api/agents/resources/prompts",
        save_body("other", "soul.md", "x"),
    )
    .await;
    expect_silent(&mut cmd_rx);

    // Referenced rollback ⇒ fan-out; unreferenced rollback ⇒ silent.
    call(
        app(state.clone()),
        "POST",
        "/api/agents/resources/prompts/pack/rollback",
        serde_json::json!({ "version": 1 }),
    )
    .await;
    expect_reload(&mut cmd_rx);
    call(
        app(state.clone()),
        "POST",
        "/api/agents/resources/prompts/other/rollback",
        serde_json::json!({ "version": 1 }),
    )
    .await;
    expect_silent(&mut cmd_rx);
}

/// DELETE refuses (409 + referencing cards) while any card points at the
/// pool; once unreferenced the whole versioned dir goes away.
#[tokio::test]
async fn delete_referenced_conflicts_then_removes() {
    let state = state().await;
    let _scoped = scoped();
    call(
        app(state.clone()),
        "POST",
        "/api/agents/resources/prompts",
        save_body("pack", "soul.md", "one"),
    )
    .await;
    call(
        app(state.clone()),
        "POST",
        "/api/agents",
        serde_json::json!({ "name": "work", "current": { "prompt": "pack" } }),
    )
    .await;

    let (status, v) = call(
        app(state.clone()),
        "DELETE",
        "/api/agents/resources/prompts/pack",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["ok"], false);
    assert_eq!(v["referenced_by"], serde_json::json!(["work"]));
    // Still there after the refusal.
    let (status, _) = call(
        app(state.clone()),
        "GET",
        "/api/agents/resources/prompts/pack/meta",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Drop the reference, then delete succeeds and removes every version.
    call(
        app(state.clone()),
        "PUT",
        "/api/agents/work",
        serde_json::json!({ "current": {} }),
    )
    .await;
    let (status, v) = call(
        app(state.clone()),
        "DELETE",
        "/api/agents/resources/prompts/pack",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (status, _) = call(
        app(state.clone()),
        "GET",
        "/api/agents/resources/prompts/pack/meta",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Unknown resource / category on delete ⇒ 404 / 400.
    let (status, _) = call(
        app(state.clone()),
        "DELETE",
        "/api/agents/resources/prompts/ghost",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(
        app(state.clone()),
        "DELETE",
        "/api/agents/resources/nope/pack",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn missing_resource_and_versions_are_404() {
    let state = state().await;
    let _scoped = scoped();
    call(
        app(state.clone()),
        "POST",
        "/api/agents/resources/prompts",
        save_body("pack", "soul.md", "one"),
    )
    .await;
    let (status, _) = call(
        app(state.clone()),
        "GET",
        "/api/agents/resources/prompts/ghost/meta",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(
        app(state.clone()),
        "PUT",
        "/api/agents/resources/prompts/ghost",
        save_body("ghost", "soul.md", "x"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Unknown version / missing file inside a real version ⇒ 404.
    for uri in [
        "/api/agents/resources/prompts/pack/versions/9/files/soul.md",
        "/api/agents/resources/prompts/pack/versions/1/files/absent.md",
        "/api/agents/resources/prompts/ghost/versions/1/files/soul.md",
    ] {
        let (status, _) = call(app(state.clone()), "GET", uri, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
    }
    // Rollback to a version never saved ⇒ 400; unknown resource ⇒ 404.
    let (status, _) = call(
        app(state.clone()),
        "POST",
        "/api/agents/resources/prompts/pack/rollback",
        Some(serde_json::json!({ "version": 7 })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(
        app(state.clone()),
        "POST",
        "/api/agents/resources/prompts/ghost/rollback",
        Some(serde_json::json!({ "version": 1 })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
