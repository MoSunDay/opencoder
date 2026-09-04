//! `/api/brain` REST contract tests, driven through the production
//! `build_app` router with a `MockChatClient`-backed brain runtime (same
//! shape as `web_envs.rs`: oneshot + JSON assertions, zero network).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::json;
use tower::ServiceExt;

use opencoder_store::{LibsqlStore, Store};

const TOKEN: &str = "sekret-token-123";

/// A valid payload with two exemplar inputs; `summary` is parameterised.
fn payload(summary: &str) -> serde_json::Value {
    json!({
        "capability_type": "tool-usage",
        "summary": summary,
        "input_desc": "a shell command request",
        "output_desc": "command stdout",
        "eng_inputs": [
            "run `cargo test -p opencoder-web`",
            "run `cargo check --workspace`"
        ],
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

/// Same shape but the brain is wired to the bail-only client — the degraded
/// `serve()` path — so embed-dependent routes must answer 502.
async fn degraded_state() -> Arc<opencoder_web::AppState> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    Arc::new(opencoder_web::AppState {
        brain: opencoder_web::api_brain::degraded_brain(store.clone()),
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

/// Create → list → get → update → delete → 404, asserting the exemplar
/// inputs survive every hop.
#[tokio::test]
async fn capability_crud_roundtrip() {
    let app = app(state().await, None);

    // POST: 201 + minted id + ordered eng_inputs.
    let (st, body) = call(
        &app,
        "POST",
        "/api/brain/capabilities",
        Some(payload("run workspace tests")),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{body}");
    assert!(body["ok"].as_bool().unwrap());
    let id = body["capability"]["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("brain-"), "id must carry the prefix: {id}");
    let eng = body["eng_inputs"].as_array().unwrap();
    assert_eq!(eng.len(), 2);
    assert_eq!(
        eng[0]["content"].as_str().unwrap(),
        "run `cargo test -p opencoder-web`"
    );
    assert_eq!(eng[1]["position"].as_i64().unwrap(), 1);

    // GET list: the entry is there with its eng_inputs intact (list items
    // keep the nested Detail shape: capability + eng_inputs).
    let (st, body) = call(&app, "GET", "/api/brain/capabilities", None).await;
    assert_eq!(st, StatusCode::OK);
    let mine = body["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["capability"]["id"].as_str() == Some(id.as_str()))
        .expect("created capability must be listed");
    assert_eq!(mine["eng_inputs"].as_array().unwrap().len(), 2);

    // GET :id single.
    let (st, body) = call(&app, "GET", &format!("/api/brain/capabilities/{id}"), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        body["capability"]["summary"].as_str().unwrap(),
        "run workspace tests"
    );

    // PUT :id replaces the summary (and re-embeds behind the scenes).
    let (st, body) = call(
        &app,
        "PUT",
        &format!("/api/brain/capabilities/{id}"),
        Some(payload("run workspace tests and lints")),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert_eq!(
        body["capability"]["summary"].as_str().unwrap(),
        "run workspace tests and lints"
    );
    let updated_at = body["capability"]["updated_at"].as_i64().unwrap();
    assert!(updated_at > 0);

    // DELETE :id then GET :id → 404.
    let (st, body) = call(
        &app,
        "DELETE",
        &format!("/api/brain/capabilities/{id}"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert!(body["ok"].as_bool().unwrap());
    let (st, _) = call(&app, "GET", &format!("/api/brain/capabilities/{id}"), None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

/// Searching with the exact composed embed text must return the capability
/// at cosine distance ~0 (the mock embedder is a pure function of the text);
/// a plain-summary query still wins top-1 but at a non-zero distance.
#[tokio::test]
async fn search_top_hit_and_exact_distance() {
    let app = app(state().await, None);
    let (_, body) = call(
        &app,
        "POST",
        "/api/brain/capabilities",
        Some(payload("run workspace tests")),
    )
    .await;
    let id = body["capability"]["id"].as_str().unwrap().to_string();

    let input: opencoder_brain::CapabilityInput =
        serde_json::from_value(payload("run workspace tests")).unwrap();
    let composed = opencoder_brain::domain::compose_embed_text(&input);
    let (st, body) = call(
        &app,
        "POST",
        "/api/brain/search",
        Some(json!({ "query": composed })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    let hits = body["hits"].as_array().unwrap();
    assert!(!hits.is_empty());
    assert_eq!(hits[0]["capability"]["id"].as_str().unwrap(), id);
    assert!(
        hits[0]["distance"].as_f64().unwrap() < 1e-6,
        "same text must embed identically: {body}"
    );

    // Summary-only query: different bytes ⇒ different mock vector, k is
    // clamped (999 → 50) without erroring.
    let (st, body) = call(
        &app,
        "POST",
        "/api/brain/search",
        Some(json!({ "query": "run workspace tests", "k": 999 })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    let hits = body["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["capability"]["id"].as_str().unwrap(), id);
    assert!(hits[0]["distance"].as_f64().unwrap() > 1e-6);
}

/// Validate rejections surface as 400 with the field name passed through.
#[tokio::test]
async fn invalid_payload_is_400() {
    let app = app(state().await, None);
    for (label, p) in [
        ("blank summary", payload("   ")),
        ("too many eng_inputs", {
            let mut p = payload("s");
            p["eng_inputs"] = serde_json::to_value(vec!["e"; 300]).unwrap();
            p
        }),
    ] {
        let (st, body) = call(&app, "POST", "/api/brain/capabilities", Some(p)).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{label}: {body}");
        assert!(
            body["error"].as_str().unwrap().contains("summary")
                || body["error"].as_str().unwrap().contains("eng_inputs")
        );
    }

    // PUT validates too — a bad payload on a real id is 400, not 502.
    let (_, created) = call(
        &app,
        "POST",
        "/api/brain/capabilities",
        Some(payload("run workspace tests")),
    )
    .await;
    let id = created["capability"]["id"].as_str().unwrap();
    let (st, _) = call(
        &app,
        "PUT",
        &format!("/api/brain/capabilities/{id}"),
        Some(payload("")),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

/// Unknown ids: GET/PUT/DELETE → 404; empty search query → 400.
#[tokio::test]
async fn missing_ids_and_empty_query() {
    let app = app(state().await, None);
    let (st, _) = call(&app, "GET", "/api/brain/capabilities/brain-nope", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, body) = call(
        &app,
        "PUT",
        "/api/brain/capabilities/brain-nope",
        Some(payload("run workspace tests")),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND, "{body}");
    // 404 comes from the typed `BrainNotFound` marker; its Display keeps the
    // historical "brain capability not found: {id}" body shape.
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("brain capability not found"),
        "{body}"
    );
    let (st, _) = call(&app, "DELETE", "/api/brain/capabilities/brain-nope", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, body) = call(
        &app,
        "POST",
        "/api/brain/search",
        Some(json!({ "query": "   " })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{body}");
}

/// The degraded serve() path: embed outage → 502 with a message that names
/// both the wrapping context and the underlying reason.
#[tokio::test]
async fn degraded_client_maps_embed_failure_to_502() {
    let app = app(degraded_state().await, None);
    let (st, body) = call(
        &app,
        "POST",
        "/api/brain/capabilities",
        Some(payload("run workspace tests")),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_GATEWAY, "{body}");
    let err = body["error"].as_str().unwrap();
    assert!(err.contains("embedding failed"), "{err}");
    assert!(err.contains("llm endpoint unavailable"), "{err}");

    let (st, body) = call(
        &app,
        "POST",
        "/api/brain/search",
        Some(json!({ "query": "anything" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_GATEWAY, "{body}");

    // Reads stay healthy in degraded mode — only embedding is down.
    let (st, body) = call(&app, "GET", "/api/brain/capabilities", None).await;
    assert_eq!(st, StatusCode::OK, "{body}");
}

/// Brain routes live under `/api`, so they inherit the HMAC signature gate.
#[tokio::test]
async fn unsigned_brain_request_is_401() {
    let app = app(state().await, Some(TOKEN.to_string()));
    let (st, _) = call(&app, "GET", "/api/brain/capabilities", None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}
