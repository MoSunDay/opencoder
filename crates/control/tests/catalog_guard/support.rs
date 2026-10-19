use axum::{body::Body, http::Request, Router};
use base64::Engine;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

pub struct Harness {
    pub state: Arc<opencoder_control::AppState>,
    pub app: Router,
    pub dir: tempfile::TempDir,
    _config: opencoder_core::config::ScopedConfigHome,
}
impl Harness {
    pub async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config = opencoder_core::config::scoped_config_home(dir.path().into());
        let work = dir.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(dir.path().join("agents")).unwrap();
        std::fs::write(work.join("opencoder.json"),json!({"agent":{"agents_dir":dir.path().join("agents")},"dag":{"binary_dir":dir.path().join("binaries")}}).to_string()).unwrap();
        let state = opencoder_control::new_state(
            work,
            dir.path().join("state"),
            Some(Arc::new(opencoder_llm::MockChatClient::new())),
        )
        .await
        .unwrap();
        let app = opencoder_control::build_app(state.clone(), None, false);
        Self {
            state,
            app,
            dir,
            _config: config,
        }
    }
    pub async fn call(&self, method: &str, path: &str, body: Value) -> (u16, Value, String) {
        call(&self.app, method, path, body, None).await
    }
}
pub async fn call(
    app: &Router,
    method: &str,
    path: &str,
    body: Value,
    etag: Option<&str>,
) -> (u16, Value, String) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(etag) = etag {
        request = request.header("if-match", etag);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status().as_u16();
    let tag = response
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        tag,
    )
}
pub fn spec(name: &str) -> Value {
    json!({"name":name,"steps":[{"name":"run","kind":{"type":"agent","prompt":"inspect the inputs"}}]})
}
pub fn binary(name: &str, payload: &str) -> Value {
    let mut bytes = vec![0; 120];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    for (i, v) in [
        (16, 2),
        (18, 62),
        (20, 1),
        (32, 64),
        (52, 64),
        (54, 56),
        (56, 1),
        (64, 1),
    ] {
        bytes[i] = v;
    }
    bytes.extend_from_slice(payload.as_bytes());
    json!({"name":name,"description":"native tool","binary_b64":base64::engine::general_purpose::STANDARD.encode(bytes)})
}
