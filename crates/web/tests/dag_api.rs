//! HTTP contract for the DAG control plane (`/api/dag/*` + `/api/nodes/dag/*`):
//! defs CRUD, dispatch/claim with spec snapshot, event upload validation,
//! SSE replay/resume, terminal status with synthetic `run_finished`, cancel
//! piggyback on the heartbeat. Store-backed (the libsql DAG impl is live);
//! harness mirrors `nodes_ops.rs`.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use opencoder_llm::MockChatClient;
use opencoder_store::{LibsqlStore, Store};
use tower::ServiceExt;

/// Two python steps with one dependency — the minimal valid workflow.
const SPEC: &str = r#"{"name":"etl-demo","steps":[
    {"name":"fetch","kind":{"type":"python","code":"x=1"}},
    {"name":"load","depends_on":["fetch"],"kind":{"type":"python","code":"y=2"}}]}"#;

/// Wrap a raw spec literal in the `DagDefUpsertRequest` envelope.
fn spec_body_of(spec: &str) -> String {
    format!(r#"{{"spec":{spec}}}"#)
}

fn spec_body() -> String {
    spec_body_of(SPEC)
}

struct Ctx {
    app: axum::Router,
    store: Arc<dyn Store>,
}

async fn app() -> Ctx {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let state = Arc::new(opencoder_web::AppState {
        brain: opencoder_web::api_brain::mock_brain(store.clone()),
        store: store.clone(),
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        team: opencoder_web::team_state::mock(),
        project: opencoder_web::ProjectService::new(),
        client_override: Some(Arc::new(MockChatClient::new())),
    });
    Ctx {
        app: opencoder_web::build_app(state, None, false),
        store,
    }
}

async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
    let resp = app.clone().oneshot(req).await.expect("router must answer");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let body = if bytes.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({}))
    };
    (status, body)
}

fn req(method: &str, uri: &str, body: Option<String>) -> Request<Body> {
    match body {
        Some(json) => Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(json)),
        None => Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty()),
    }
    .unwrap()
}

async fn register(app: &axum::Router, name: &str) -> String {
    let (_, b) = send(
        app,
        req(
            "POST",
            "/api/nodes/register",
            Some(format!(r#"{{"name":"{name}"}}"#)),
        ),
    )
    .await;
    b["node_id"].as_str().unwrap().into()
}

/// Upsert the sample def; returns its (stable) id.
async fn upsert_def(app: &axum::Router) -> String {
    let (s, b) = send(app, req("POST", "/api/dag/defs", Some(spec_body()))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    b["id"].as_str().unwrap().into()
}

async fn dispatch(app: &axum::Router, def_id: &str, node_id: Option<&str>) -> String {
    let body = match node_id {
        Some(n) => format!(r#"{{"node_id":"{n}"}}"#),
        None => "{}".to_string(),
    };
    let (s, b) = send(
        app,
        req(
            "POST",
            &format!("/api/dag/defs/{def_id}/dispatch"),
            Some(body),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    b["run_id"].as_str().unwrap().into()
}

/// Claim for `node_id`; `None` models the 204 idle answer.
async fn claim(app: &axum::Router, node_id: &str) -> Option<serde_json::Value> {
    let (s, b) = send(
        app,
        req(
            "GET",
            &format!("/api/nodes/dag/claim?node_id={node_id}"),
            None,
        ),
    )
    .await;
    if s == StatusCode::NO_CONTENT {
        return None;
    }
    assert_eq!(s, StatusCode::OK, "{b}");
    Some(b)
}

/// Upload one event; returns the raw (status, body).
async fn upload(
    app: &axum::Router,
    rid: &str,
    events: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    send(
        app,
        req(
            "POST",
            &format!("/api/nodes/dag/runs/{rid}/events"),
            Some(serde_json::json!({ "run_id": rid, "events": events }).to_string()),
        ),
    )
    .await
}

// ── defs CRUD ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn defs_crud_upsert_keeps_id_and_delete_404s() {
    let ctx = app().await;
    let id = upsert_def(&ctx.app).await;

    // Re-publish under the same name: the FIRST row's id survives.
    let (_, again) = send(&ctx.app, req("POST", "/api/dag/defs", Some(spec_body()))).await;
    assert_eq!(again["id"].as_str().unwrap(), id);
    assert!(again["updated_at"].as_i64().unwrap() >= again["created_at"].as_i64().unwrap());

    let (s, list) = send(&ctx.app, req("GET", "/api/dag/defs", None)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        list.as_array().unwrap().len(),
        1,
        "upsert by name, not append"
    );
    assert_eq!(list[0]["spec"]["steps"].as_array().unwrap().len(), 2);

    let (s, one) = send(&ctx.app, req("GET", &format!("/api/dag/defs/{id}"), None)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(one["name"], "etl-demo");

    let (s, _) = send(
        &ctx.app,
        req("DELETE", &format!("/api/dag/defs/{id}"), None),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = send(&ctx.app, req("GET", &format!("/api/dag/defs/{id}"), None)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = send(
        &ctx.app,
        req("DELETE", &format!("/api/dag/defs/{id}"), None),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn invalid_spec_is_400_with_the_problem_list() {
    let ctx = app().await;
    let bad = r#"{"name":"x","steps":[]}"#;
    let (s, b) = send(
        &ctx.app,
        req("POST", "/api/dag/defs", Some(spec_body_of(bad))),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(
        b["error"]
            .as_str()
            .unwrap()
            .contains("spec.steps must not be empty"),
        "{b}"
    );

    let bad_slug =
        r#"{"name":"x","steps":[{"name":"Bad Slug","kind":{"type":"python","code":"1"}}]}"#;
    let (s, b) = send(
        &ctx.app,
        req("POST", "/api/dag/defs", Some(spec_body_of(bad_slug))),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(
        b["error"].as_str().unwrap().contains("not a valid slug"),
        "{b}"
    );

    let (_, list) = send(&ctx.app, req("GET", "/api/dag/defs", None)).await;
    assert_eq!(
        list.as_array().unwrap().len(),
        0,
        "rejected specs are not stored"
    );
}

// ── dispatch + claim ───────────────────────────────────────────────────────

#[tokio::test]
async fn dispatch_unknown_def_404_and_unknown_node_400() {
    let ctx = app().await;
    let (s, b) = send(
        &ctx.app,
        req("POST", "/api/dag/defs/01GHOST/dispatch", Some("{}".into())),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{b}");

    let def_id = upsert_def(&ctx.app).await;
    let (s, b) = send(
        &ctx.app,
        req(
            "POST",
            &format!("/api/dag/defs/{def_id}/dispatch"),
            Some(r#"{"node_id":"01NOPE"}"#.into()),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(
        b["error"].as_str().unwrap().contains("does not exist"),
        "{b}"
    );
}

#[tokio::test]
async fn claim_returns_spec_snapshot_and_second_claim_is_204() {
    let ctx = app().await;
    let node = register(&ctx.app, "worker-1").await;
    let def_id = upsert_def(&ctx.app).await;
    let rid = dispatch(&ctx.app, &def_id, Some(&node)).await;

    let claimed = claim(&ctx.app, &node).await.expect("run was due");
    assert_eq!(claimed["run_id"].as_str().unwrap(), rid);
    assert_eq!(claimed["dag_id"].as_str().unwrap(), def_id);
    let steps = claimed["spec"]["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 2, "claim carries the spec snapshot");
    assert_eq!(steps[1]["depends_on"][0], "fetch");

    // Single-active-run policy: the busy node gets nothing more.
    assert!(claim(&ctx.app, &node).await.is_none());

    let (s, run) = send(&ctx.app, req("GET", &format!("/api/dag/runs/{rid}"), None)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(run["status"], "running");
    assert_eq!(run["node_id"], node.as_str());
    assert!(run["claimed_at"].is_i64());

    // Newest-first listing sees the run.
    let (_, runs) = send(&ctx.app, req("GET", "/api/dag/runs?limit=5", None)).await;
    assert_eq!(runs.as_array().unwrap().len(), 1);
}

// ── event upload validation ────────────────────────────────────────────────

#[tokio::test]
async fn event_upload_validates_run_kind_and_run_id() {
    let ctx = app().await;
    let node = register(&ctx.app, "worker-2").await;
    let def_id = upsert_def(&ctx.app).await;
    let rid = dispatch(&ctx.app, &def_id, Some(&node)).await;
    let _ = claim(&ctx.app, &node).await;

    let (s, b) = upload(&ctx.app, "01GHOST", serde_json::json!([])).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{b}");

    let mismatch = serde_json::json!([{"kind":"run_started","at_ms":1}]);
    let resp = ctx
        .app
        .clone()
        .oneshot(req(
            "POST",
            &format!("/api/nodes/dag/runs/{rid}/events"),
            Some(serde_json::json!({"run_id":"other","events":mismatch}).to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let (s, b) = upload(
        &ctx.app,
        &rid,
        serde_json::json!([{"kind":"step_exploded","step":"fetch","at_ms":1}]),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(
        b["error"]
            .as_str()
            .unwrap()
            .contains("unknown dag event kind"),
        "{b}"
    );

    // Rejected batches leave nothing behind.
    let persisted = ctx.store.dag_events_after(&rid, 0, 100).await.unwrap();
    assert!(persisted.is_empty());

    let (s, b) = upload(
        &ctx.app,
        &rid,
        serde_json::json!([
            {"kind":"run_started","at_ms":1},
            {"kind":"step_started","step":"fetch","at_ms":2}
        ]),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["accepted"], 2);
}

/// A terminal run closes its event stream: uploads after the terminal status
/// report are 409'd and nothing new is persisted (the only frame the
/// terminal move adds is the store's own synthetic `run_finished`).
#[tokio::test]
async fn events_rejected_after_terminal_status() {
    let ctx = app().await;
    let node = register(&ctx.app, "worker-3").await;
    let def_id = upsert_def(&ctx.app).await;
    let rid = dispatch(&ctx.app, &def_id, Some(&node)).await;
    let _ = claim(&ctx.app, &node).await;

    // Live run: uploads land.
    let (s, b) = upload(
        &ctx.app,
        &rid,
        serde_json::json!([{"kind":"run_started","at_ms":1}]),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let before = ctx
        .store
        .dag_events_after(&rid, 0, 100)
        .await
        .unwrap()
        .len();

    // Terminal report, same envelope the node sends.
    let (s, b) = send(
        &ctx.app,
        req(
            "POST",
            &format!("/api/nodes/dag/runs/{rid}/status"),
            Some(format!(r#"{{"run_id":"{rid}","status":"done"}}"#)),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");

    // Post-terminal upload: 409, and the stream must not grow.
    let (s, b) = upload(
        &ctx.app,
        &rid,
        serde_json::json!([{"kind":"step_done","step":"fetch","at_ms":2}]),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert!(
        b["error"].as_str().unwrap().contains("event stream closed"),
        "{b}"
    );
    let after = ctx.store.dag_events_after(&rid, 0, 100).await.unwrap();
    assert_eq!(
        after.len(),
        before + 1,
        "only the synthetic run_finished frame was appended"
    );
    assert_eq!(after.last().unwrap().kind, "run_finished");
    assert_eq!(after.last().unwrap().payload["status"], "done");
}
