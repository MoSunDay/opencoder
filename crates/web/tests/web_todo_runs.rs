//! `/api/todo/templates/:n/:v/run` + `/api/todo/workflows` e2e contract
//! tests: run spawn → terminal status, observability endpoints, the SSE
//! event tail (closes after the terminal frame) and interrupt/resume
//! semantics against the real `opencoder_todos::Runtime` + MockChatClient.
//!
//! Mock contract pinned from source before writing assertions:
//! - `MockChatClient::with_default(fail-decision)` makes every parent LLM
//!   call return a valid `{"operation":"fail",...}` decision, so a spawned
//!   run deterministically reaches `workflow_failed` (terminal).
//! - `queue_hang` parks the run's first LLM call, giving a stable `running`
//!   workflow for the resume-conflict / interrupt paths.
//! - Terminal workflows: resume answers an explicit `{ok, terminal}` 200
//!   without spawning a runtime, interrupt is a 409; `running` resume is
//!   refused with a 409 pre-check (two drivers would fight the generation
//!   CAS).

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::{get, post};
use axum::Router;
use tower::ServiceExt;

use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{LibsqlStore, Store};
use opencoder_web::{api_todo_envs as envs, api_todo_runs as runs, api_todo_templates as tpl};

static GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn share() -> (tokio::sync::MutexGuard<'static, ()>, std::path::PathBuf) {
    let guard = GATE.lock().await;
    let root = std::env::temp_dir().join(format!("oc-web-todo-run-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let agents =
        std::env::temp_dir().join(format!("oc-web-todo-run-agents-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&agents).unwrap();
    opencoder_core::set_share_dir_override(Some(root.clone()));
    opencoder_core::agent::set_agents_dir_override(Some(agents));
    (guard, root)
}

fn app(state: Arc<opencoder_web::AppState>) -> Router {
    Router::new()
        .route(
            "/api/todo/templates/:name/:version/run",
            post(runs::run_template),
        )
        .route("/api/todo/workflows", get(runs::list_workflows))
        .route("/api/todo/workflows/:id", get(runs::get_workflow))
        .route(
            "/api/todo/workflows/:id/interrupt",
            post(runs::interrupt_workflow),
        )
        .route(
            "/api/todo/workflows/:id/resume",
            post(runs::resume_workflow),
        )
        .route(
            "/api/todo/workflows/:id/events",
            get(opencoder_web::todo_hub::workflow_events),
        )
        .route("/api/todo/templates", post(tpl::create_template))
        .route(
            "/api/todo/templates/:name/:version/env.json",
            get(tpl::get_env_binding).put(tpl::put_env_binding),
        )
        .route("/api/todo/tools", get(envs::list_tools))
        .with_state(state)
}

async fn state(client: Arc<MockChatClient>) -> Arc<opencoder_web::AppState> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let workdir = std::env::temp_dir().join(format!("oc-web-run-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workdir).ok();
    Arc::new(opencoder_web::AppState {
        client_override: Some(client as Arc<dyn ChatStream>),
        store: store.clone(),
        workdir,
        handles: opencoder_web::handle::new_handle_map(),
        brain: opencoder_web::api_brain::mock_brain(store.clone()),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        team: opencoder_web::team_state::mock(),
        project: opencoder_web::ProjectService::new(),
    })
}

/// Deterministic terminal driver: every LLM call decides `fail`, which the
/// domain layer accepts unconditionally → `workflow_failed`.
fn fail_default() -> Arc<MockChatClient> {
    Arc::new(
        MockChatClient::new().with_default(vec![LlmEvent::Completed {
            text: r#"{"operation":"fail","reason":"web e2e deterministic terminal"}"#.into(),
            tool_calls: Vec::new(),
            usage: None,
        }]),
    )
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

fn spec() -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "id": "wf-1",
        "name": "demo",
        "objective": "ship it",
        "todos": [{
            "id": "t1", "title": "T1", "requirement_background": "bg", "instructions": "i",
            "agent": "act", "acceptance": { "criteria": "c" },
        }],
        "metadata": {}
    })
}

/// Seed template + env (with a resolvable tool file) + binding on disk.
fn seed_share(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("agent").join("tools").join("v3")).unwrap();
    std::fs::write(root.join("agent/tools/v3/ffmpeg"), b"#!/bin/sh\n").unwrap();
    let env_dir = root.join("env").join("dev");
    std::fs::create_dir_all(&env_dir).unwrap();
    std::fs::write(
        env_dir.join("context.json"),
        serde_json::json!({"name": "dev", "tools": ["/agent/tools/v3/ffmpeg"], "env_vars": {}})
            .to_string(),
    )
    .unwrap();
}

/// Poll the workflow record until its status is terminal (or fail on timeout).
async fn wait_terminal(state: &Arc<opencoder_web::AppState>, id: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (status, v) = call(
            app(state.clone()),
            "GET",
            &format!("/api/todo/workflows/{id}"),
            None,
        )
        .await;
        if status == StatusCode::OK {
            if let Some(s) = v["workflow"]["status"].as_str() {
                if s == "completed" || s == "failed" {
                    return s.to_string();
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "workflow {id} never reached terminal: {v}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// R-1: run a bound template → deterministic terminal state, visible through
/// every observability endpoint; SSE replays history and closes; interrupt
/// refuses terminal (409); resume on terminal answers an explicit
/// `{ok, terminal}` no-op 200 without spawning a runtime.
#[tokio::test]
async fn run_reaches_terminal_and_is_observable() {
    let (_g, root) = share().await;
    let state = state(fail_default()).await;
    let a = || app(state.clone());
    seed_share(&root);

    let (status, v) = call(
        a(),
        "POST",
        "/api/todo/templates",
        Some(serde_json::json!({"name": "demo", "spec": spec()})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (status, v) = call(
        a(),
        "PUT",
        "/api/todo/templates/demo/v1/env.json",
        Some(serde_json::json!({"env": "dev"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");

    let (status, v) = call(a(), "POST", "/api/todo/templates/demo/v1/run", None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let id = v["workflow_id"].as_str().unwrap().to_string();
    assert!(id.starts_with("todos-"), "{id}");

    // Mock always decides `fail` → deterministic terminal `failed`.
    assert_eq!(wait_terminal(&state, &id).await, "failed");

    let (status, v) = call(a(), "GET", "/api/todo/workflows?limit=10", None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert!(v["workflows"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w["id"] == id));

    let (status, v) = call(a(), "GET", &format!("/api/todo/workflows/{id}"), None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["workflow"]["status"], "failed");
    assert_eq!(
        v["items"].as_array().unwrap().len(),
        1,
        "one TODO projection"
    );
    assert_eq!(v["workflow"]["spec_json"]["metadata"]["env"], "dev");
    assert_eq!(
        v["workflow"]["spec_json"]["metadata"]["env_tools"],
        serde_json::json!(["/agent/tools/v3/ffmpeg"])
    );

    // Interrupt refuses terminal workflows with 409 (a settled outcome
    // cannot be parked; previously this surfaced as a runtime 500).
    let (status, v) = call(
        a(),
        "POST",
        &format!("/api/todo/workflows/{id}/interrupt"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    assert!(v["error"].as_str().unwrap().contains("终态"));

    // Resume on a terminal workflow: accepted no-op, with an explicit
    // `terminal` marker so callers can tell it apart from a real takeover.
    let (status, v) = call(
        a(),
        "POST",
        &format!("/api/todo/workflows/{id}/resume"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["ok"], true);
    assert_eq!(v["terminal"], "failed");

    // SSE: 404 before the stream opens for an unknown workflow.
    let (status, _) = call(a(), "GET", "/api/todo/workflows/todos-none/events", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // SSE on the terminal workflow: replay from cursor 0 then close after
    // the terminal frame — to_bytes completes because the stream ends.
    let resp = a()
        .oneshot(
            Request::builder()
                .uri(format!("/api/todo/workflows/{id}/events?after=0"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .starts_with("text/event-stream"));
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(text.contains("event: workflow_created"), "{text}");
    assert!(text.contains("event: workflow_failed"), "{text}");
    assert!(text.contains("id:"), "seq ids act as Last-Event-ID cursors");
}

/// R-2: a `running` workflow refuses resume (409) until an interrupt parks
/// it (`suspended`); interrupt then succeeds.
#[tokio::test]
async fn resume_conflicts_while_running_until_interrupt() {
    let (_g, root) = share().await;
    let mock = Arc::new(MockChatClient::new());
    let state = state(mock.clone()).await;
    let a = || app(state.clone());
    seed_share(&root);

    let (status, v) = call(
        a(),
        "POST",
        "/api/todo/templates",
        Some(serde_json::json!({"name": "demo", "spec": spec()})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");

    // Park the parent decision call so the workflow stays `running`.
    mock.queue_hang(Arc::new(tokio::sync::Notify::new()));
    let (status, v) = call(a(), "POST", "/api/todo/templates/demo/v1/run", None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let id = v["workflow_id"].as_str().unwrap().to_string();

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (_, v) = call(a(), "GET", &format!("/api/todo/workflows/{id}"), None).await;
        if v["workflow"]["status"] == "running" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "workflow {id} never started: {v}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let (status, v) = call(
        a(),
        "POST",
        &format!("/api/todo/workflows/{id}/resume"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    assert!(v["error"].as_str().unwrap().contains("仍在运行"));

    let (status, v) = call(
        a(),
        "POST",
        &format!("/api/todo/workflows/{id}/interrupt"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "suspended");

    // Post-interrupt resume is accepted (the parked spawn may still hold the
    // mock open; no state assertion — the 409/suspended contract above is
    // what this test pins).
    let (status, v) = call(
        a(),
        "POST",
        &format!("/api/todo/workflows/{id}/resume"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
}
