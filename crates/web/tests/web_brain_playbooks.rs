//! `/api/brain/playbooks` REST contract tests, driven through the production
//! `build_app` router (same shape as `web_brain.rs`: oneshot + JSON
//! assertions, zero network). Playbooks carry no embeddings, so the mock
//! brain is only the store carrier — the contract under test is the CRUD
//! mapping: payload rejections → 400 (joined verbatim), unknown id → 404,
//! the minted id/echoed spec shape, and update/delete semantics.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::json;
use tower::ServiceExt;

use opencoder_store::{LibsqlStore, Store};

const TOKEN: &str = "sekret-token-123";

/// A valid playbook payload: serial agent chain a → b.
fn payload(name: &str) -> serde_json::Value {
    json!({
        "name": name,
        "steps": [
            {"name": "a", "target": {"kind": "agent", "agent": "act"}, "prompt": "do a"},
            {
                "name": "b",
                "depends_on": ["a"],
                "target": {"kind": "agent", "agent": "act"},
                "prompt": "do b"
            }
        ]
    })
}

async fn state() -> Arc<opencoder_web::AppState> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    Arc::new(opencoder_web::AppState {
        brain: opencoder_web::api_brain::mock_brain(store.clone()),
        store,
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        team: opencoder_web::team_state::mock(),
        project: opencoder_web::ProjectService::new(),
        client_override: None,
    })
}

fn app(state: Arc<opencoder_web::AppState>, token: Option<String>) -> Router {
    opencoder_web::build_app(state, token, false)
}

async fn call(
    app: &Router,
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
    let resp = app.clone().oneshot(req).await.expect("router must answer");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 4 << 20)
        .await
        .unwrap();
    let body = if bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&bytes).unwrap_or(json!({}))
    };
    (status, body)
}

/// Create → list → get → update (rename) → delete → 404, asserting the
/// minted id and the echoed spec at every hop.
#[tokio::test]
async fn playbook_crud_roundtrip() {
    let app = app(state().await, None);

    // POST: 201 + minted playbook-{ULID} id + spec echoing the steps.
    let (st, body) = call(
        &app,
        "POST",
        "/api/brain/playbooks",
        Some(payload("ci 修复")),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{body}");
    assert!(body["ok"].as_bool().unwrap());
    let id = body["playbook"]["id"].as_str().unwrap().to_string();
    assert!(
        id.starts_with("playbook-"),
        "id must carry the prefix: {id}"
    );
    let spec = &body["spec"];
    assert_eq!(spec["id"].as_str().unwrap(), id);
    assert_eq!(spec["name"].as_str().unwrap(), "ci 修复");
    assert_eq!(spec["origin"]["kind"].as_str().unwrap(), "fixed");
    assert_eq!(spec["steps"].as_array().unwrap().len(), 2);
    assert_eq!(spec["steps"][1]["depends_on"][0].as_str().unwrap(), "a");

    // GET list: the record shape (spec kept as the opaque spec_json string).
    let (st, body) = call(&app, "GET", "/api/brain/playbooks", None).await;
    assert_eq!(st, StatusCode::OK, "{body}");
    let books = body["playbooks"].as_array().unwrap();
    assert_eq!(books.len(), 1);
    assert_eq!(books[0]["id"].as_str().unwrap(), id);
    assert!(books[0]["spec_json"].as_str().unwrap().contains("\"b\""));

    // GET one: record + decoded spec.
    let (st, body) = call(&app, "GET", &format!("/api/brain/playbooks/{id}"), None).await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert_eq!(body["playbook"]["id"].as_str().unwrap(), id);
    assert_eq!(body["spec"]["steps"].as_array().unwrap().len(), 2);

    // PUT: rename only — id and step graph preserved, name replaced.
    let (st, body) = call(
        &app,
        "PUT",
        &format!("/api/brain/playbooks/{id}"),
        Some(payload("ci 修复 v2")),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert!(body["ok"].as_bool().unwrap());
    assert_eq!(body["spec"]["name"].as_str().unwrap(), "ci 修复 v2");
    assert_eq!(body["spec"]["id"].as_str().unwrap(), id);

    // DELETE: ok, then the second delete is a 404.
    let (st, body) = call(&app, "DELETE", &format!("/api/brain/playbooks/{id}"), None).await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"].as_str().unwrap(), id);
    let (st, body) = call(&app, "DELETE", &format!("/api/brain/playbooks/{id}"), None).await;
    assert_eq!(st, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        body["error"].as_str().unwrap(),
        format!("brain playbook not found: {id}")
    );
}

/// POST with domain-invalid steps: every problem is aggregated into one
/// joined 400 message, passed through verbatim.
#[tokio::test]
async fn playbook_create_rejects_invalid_steps_with_400() {
    let app = app(state().await, None);
    // Two problems: "b" depends on the missing step "zz", and "c" is a
    // duplicate of "a"'s name... (name uniqueness) — use the missing dep +
    // an empty prompt to get two distinct messages in one report.
    let bad = json!({
        "name": "坏剧本",
        "steps": [
            {"name": "a", "target": {"kind": "agent", "agent": "act"}, "prompt": ""},
            {"name": "a", "depends_on": ["zz"], "target": {"kind": "agent", "agent": "act"}, "prompt": "p"}
        ]
    });
    let (st, body) = call(&app, "POST", "/api/brain/playbooks", Some(bad)).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{body}");
    let err = body["error"].as_str().unwrap();
    assert!(err.contains("zz"), "missing dependency named: {err}");
    assert!(err.contains(';'), "problems aggregated: {err}");

    // Nothing was persisted.
    let (st, body) = call(&app, "GET", "/api/brain/playbooks", None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(body["playbooks"].as_array().unwrap().is_empty());
}

/// Unknown ids answer the historical 404 shape on every method.
#[tokio::test]
async fn playbook_unknown_id_is_404() {
    let app = app(state().await, None);
    for method in ["GET", "PUT", "DELETE"] {
        let (st, body) = call(
            &app,
            method,
            "/api/brain/playbooks/playbook-nope",
            Some(payload("任何名字")),
        )
        .await;
        assert_eq!(st, StatusCode::NOT_FOUND, "{method} {body}");
        assert_eq!(
            body["error"].as_str().unwrap(),
            "brain playbook not found: playbook-nope"
        );
    }
}

/// PUT validates the payload before touching the store: an invalid body on
/// an existing playbook is still a 400.
#[tokio::test]
async fn playbook_update_rejects_invalid_payload() {
    let app = app(state().await, None);
    let (st, body) = call(&app, "POST", "/api/brain/playbooks", Some(payload("ok"))).await;
    assert_eq!(st, StatusCode::CREATED, "{body}");
    let id = body["playbook"]["id"].as_str().unwrap().to_string();

    let bad = json!({
        "name": "空步骤",
        "steps": [
            {"name": "b", "depends_on": ["a"], "target": {"kind": "agent", "agent": "act"}, "prompt": "p"}
        ]
    });
    let (st, body) = call(
        &app,
        "PUT",
        &format!("/api/brain/playbooks/{id}"),
        Some(bad),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains('a'), "{}", body);

    // The stored playbook is untouched.
    let (st, body) = call(&app, "GET", &format!("/api/brain/playbooks/{id}"), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["spec"]["name"].as_str().unwrap(), "ok");
}

/// The token gate applies to the playbook surface like every other route.
#[tokio::test]
async fn playbook_routes_require_token_when_configured() {
    let app = app(state().await, Some(TOKEN.to_string()));
    let (st, _) = call(&app, "GET", "/api/brain/playbooks", None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _) = call(&app, "GET", "/api/brain/playbooks", None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}
