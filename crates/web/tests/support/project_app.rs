//! Shared harness for the `/api/project/*` integration tests: a full
//! `build_app` router (signature middleware ON) over an initialized
//! `ProjectService` backed by one in-memory libsql store and a script-queue
//! `MockChatClient`, plus the signed-oneshot call helper and run-polling
//! utilities. Used by `tests/web_project.rs` and `tests/web_project_runs.rs`.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use axum::http::StatusCode;
use axum::Router;
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient};
use opencoder_store::{LibsqlStore, ProjectStore, Store};
use serde_json::{json, Value};

use super::signed_req;
use tower::ServiceExt;

pub const TOKEN: &str = "project-test-token";

/// Full app + initialized ProjectService. `_dir` keeps the temp workdir (the
/// plan/execute background runs work against it) alive for the whole test.
pub struct Harness {
    pub app: Router,
    pub mock: Arc<MockChatClient>,
    _dir: tempfile::TempDir,
}

pub async fn harness() -> Harness {
    let libsql = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let store: Arc<dyn Store> = libsql.clone();
    let projects: Arc<dyn ProjectStore> = libsql.clone();
    let mock = Arc::new(MockChatClient::new());
    let client: Arc<dyn ChatStream> = mock.clone();
    let dir = tempfile::tempdir().unwrap();
    let project = opencoder_web::ProjectService::new();
    project
        .init(
            store.clone(),
            projects,
            dir.path().to_path_buf(),
            Some(client),
        )
        .await
        .unwrap();
    let state = Arc::new(opencoder_web::AppState {
        client_override: Some(mock.clone() as Arc<dyn ChatStream>),
        brain: opencoder_web::api_brain::mock_brain(store.clone()),
        store,
        workdir: dir.path().to_path_buf(),
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        team: opencoder_web::team_state::mock(),
        project,
    });
    Harness {
        app: opencoder_web::build_app(state, Some(TOKEN.into()), false),
        mock,
        _dir: dir,
    }
}

/// Signed oneshot JSON call. The signature covers path+query verbatim, so
/// callers must pass the exact URI they request.
pub async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let req = signed_req(method, uri, TOKEN, body.map(|v| v.to_string()));
    let resp = app.clone().oneshot(req).await.expect("router must answer");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 4 << 20)
        .await
        .unwrap();
    let v = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, v)
}

pub fn done(text: &str) -> Vec<LlmEvent> {
    vec![LlmEvent::Completed {
        text: text.into(),
        tool_calls: Vec::new(),
        usage: None,
    }]
}

pub fn tool_turn(text: &str, command: &str) -> Vec<LlmEvent> {
    vec![LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![CompletedToolCall {
            id: "t1".into(),
            name: "bash".into(),
            input: json!({ "command": command }),
        }],
        usage: None,
    }]
}

/// Poll a GET until `probe(body)` holds (25ms interval, 10s cap) — runs
/// finish in spawned backgrounds, so their effects land asynchronously.
/// Each poll appends a `?_poll=N` param: the signature covers path+query+ts,
/// and two polls of the same uri can land in one millisecond (e.g. the last
/// poll of one wait straight into the first of the next), which the replay
/// cache rightly 409s. Handlers ignore unknown query params.
pub async fn wait_until(
    app: &Router,
    uri: &str,
    what: &str,
    probe: impl Fn(&Value) -> bool,
) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    for poll in 0u64.. {
        let uniq = format!(
            "{uri}{}_poll={poll}",
            if uri.contains('?') { '&' } else { '?' }
        );
        let (status, v) = call(app, "GET", &uniq, None).await;
        assert_eq!(status, StatusCode::OK, "poll {uri}: {v}");
        if probe(&v) {
            return v;
        }
        assert!(
            Instant::now() < deadline,
            "condition not met: {what} (last: {v})"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    unreachable!("poll loop is unbounded until the deadline assert fires")
}

/// Extract one todo row from a `GET /api/project/todos` body.
pub fn todo_row(list: &Value, id: &str) -> Value {
    list["todos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .cloned()
        .unwrap()
}
