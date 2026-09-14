//! Shared harness for the `/api/dag/wasm` integration tests
//! (`tests/web_dag_wasm.rs`, `tests/web_dag_wasm_errors.rs`): thin
//! router + oneshot (same shape as `web_agents.rs`). No scope
//! middleware runs on this router, so the pool root comes from the
//! process-global wasm-dir override — every test holds ONE static lock
//! for its whole body.

use std::sync::{Arc, Mutex, MutexGuard};

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::routing::{get, post};
use axum::Router;
use tower::ServiceExt;

use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_store::{LibsqlStore, Store};
use sha2::{Digest, Sha256};

/// Serializes tests that touch the process-global wasm-pool override.
pub static OVERRIDE_LOCK: Mutex<()> = Mutex::new(());

/// A minimal valid module: 8-byte wasm header + payload. The pool layer
/// only checks the header (magic + version 1), never runs the module.
pub const MODULE: &[u8] = b"\0asm\x01\0\0\0rest-bytes";
pub const MODULE_V2: &[u8] = b"\0asm\x01\0\0\0second-cut";

/// Point the wasm pool root at a fresh tempdir under the override lock;
/// the guard must be held across every pool call in the test body.
pub fn scoped() -> (tempfile::TempDir, MutexGuard<'static, ()>) {
    let dir = tempfile::tempdir().unwrap();
    let guard = OVERRIDE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    opencoder_dag_wasm::set_wasm_dir_override(Some(dir.path().to_path_buf()));
    (dir, guard)
}

pub fn app(state: Arc<opencoder_web::AppState>) -> Router {
    Router::new()
        .route(
            "/api/dag/wasm",
            get(opencoder_web::api_dag_wasm::list).post(opencoder_web::api_dag_wasm::create),
        )
        .route(
            "/api/dag/wasm/:name",
            get(opencoder_web::api_dag_wasm::get)
                .put(opencoder_web::api_dag_wasm::put_version)
                .delete(opencoder_web::api_dag_wasm::delete),
        )
        .route(
            "/api/dag/wasm/:name/rollback",
            post(opencoder_web::api_dag_wasm::rollback),
        )
        .route(
            "/api/dag/wasm/:name/versions/:v/wasm.bin",
            get(opencoder_web::api_dag_wasm::download),
        )
        .with_state(state)
}

pub async fn state() -> Arc<opencoder_web::AppState> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let workdir = std::env::temp_dir().join(format!("oc-web-dag-wasm-{}", uuid::Uuid::new_v4()));
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

pub fn b64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub async fn call(
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
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 26)
        .await
        .unwrap();
    let v = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, v)
}

/// Binary GET (wasm.bin downloads): status + headers + full body.
pub async fn call_raw(app: Router, method: &str, uri: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
    let resp = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 26)
        .await
        .unwrap()
        .to_vec();
    (status, headers, bytes)
}

/// Seed a pool at v1 (panics on anything but 201).
pub async fn create(router: Router, name: &str) {
    let (status, v) = call(
        router,
        "POST",
        "/api/dag/wasm",
        Some(serde_json::json!({ "name": name, "wasm_b64": b64(MODULE) })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "seed create failed: {v}");
}
