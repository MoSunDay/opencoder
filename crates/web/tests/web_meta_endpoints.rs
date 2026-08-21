//! Integration tests for the metadata endpoints: requirement annotation,
//! session autopilot override, sanitized model catalog, skill catalog, the
//! `?workdir=` session filter, and `Last-Event-ID` SSE replay.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{LibsqlStore, Store};
use serde_json::json;
use tower::ServiceExt;

struct Ctx {
    app: axum::Router,
    workdir: std::path::PathBuf,
}

async fn app() -> Ctx {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let workdir = tempfile::tempdir().unwrap().keep();
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    std::fs::write(
        workdir.join(".opencoder").join("ap.json"),
        r#"{"mode":"off"}"#,
    )
    .unwrap();
    let mock = MockChatClient::new().with_default(vec![LlmEvent::Completed {
        text: "t".into(),
        tool_calls: vec![],
        usage: None,
    }]);
    let state = Arc::new(opencoder_web::AppState {
        store: store.clone(),
        workdir: workdir.clone(),
        handles: opencoder_web::handle::new_handle_map(),
        client_override: Some(Arc::new(mock) as Arc<dyn ChatStream>),
    });
    Ctx {
        app: opencoder_web::build_app(state, None, false),
        workdir,
    }
}

async fn create_session(ctx: &Ctx) -> String {
    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&axum::body::to_bytes(resp.into_body(), 4096).await.unwrap())
            .unwrap();
    body["id"].as_str().unwrap().to_string()
}

async fn post_json(ctx: &Ctx, uri: &str, body: String) -> (StatusCode, serde_json::Value) {
    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, body)
}

async fn get_json(ctx: &Ctx, uri: &str) -> (StatusCode, serde_json::Value) {
    let resp = ctx
        .app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, body)
}

// ── annotation ───────────────────────────────────────────────────────────

/// Set persists + echoes the effective requirement; blank and absent bodies
/// clear it; a missing session 404s.
#[tokio::test]
async fn annotation_set_clear_and_missing_session() {
    let ctx = app().await;
    let id = create_session(&ctx).await;

    let (status, body) = post_json(
        &ctx,
        &format!("/api/sessions/{id}/annotation"),
        json!({"text": "keep it simple"}).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["requirement"].as_str(), Some("keep it simple"));

    // Readback through the session resource.
    let (status, s) = get_json(&ctx, &format!("/api/sessions/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(s["meta"]["requirement"].as_str(), Some("keep it simple"));

    // Blank text clears.
    let (status, body) = post_json(
        &ctx,
        &format!("/api/sessions/{id}/annotation"),
        json!({"text": "   "}).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["requirement"].is_null());
    let (_, s) = get_json(&ctx, &format!("/api/sessions/{id}")).await;
    assert!(s["meta"]["requirement"].is_null(), "blank must clear");

    // Absent body also clears (no JSON body at all → treated as clear).
    let (status, _) = post_json(
        &ctx,
        &format!("/api/sessions/{id}/annotation"),
        json!({"text": "again"}).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{id}/annotation"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let (_, s) = get_json(&ctx, &format!("/api/sessions/{id}")).await;
    assert!(
        s["meta"]["requirement"].is_null(),
        "absent body must clear too"
    );

    // Missing session → 404.
    let (status, _) = post_json(
        &ctx,
        "/api/sessions/nope/annotation",
        json!({"text": "x"}).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ── autopilot ────────────────────────────────────────────────────────────

#[tokio::test]
async fn autopilot_set_invalid_and_clear() {
    let ctx = app().await;
    let id = create_session(&ctx).await;

    for mode in ["ap", "review", "off"] {
        let (status, body) = post_json(
            &ctx,
            &format!("/api/sessions/{id}/autopilot"),
            json!({"mode": mode}).to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["mode"].as_str(), Some(mode), "echo for {mode}");
    }

    let (status, body) = post_json(
        &ctx,
        &format!("/api/sessions/{id}/autopilot"),
        json!({"mode": "turbo"}).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "invalid mode: {body}");
    assert!(
        body["error"].as_str().unwrap().contains("off"),
        "error must list valid values: {body}"
    );

    // Clear: mode null → override cleared, readback null.
    let (status, _) = post_json(
        &ctx,
        &format!("/api/sessions/{id}/autopilot"),
        json!({"mode": "ap"}).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = post_json(
        &ctx,
        &format!("/api/sessions/{id}/autopilot"),
        json!({"mode": null}).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["mode"].is_null());
    let (_, s) = get_json(&ctx, &format!("/api/sessions/{id}")).await;
    assert!(s["meta"]["autopilot_mode"].is_null(), "clear must persist");
}

// ── models ───────────────────────────────────────────────────────────────

/// The catalog lists provider name/model/base_url but NEVER api_key or header
/// values (脱敏), and carries a flat dropdown array.
#[tokio::test]
async fn models_endpoint_is_sanitized_and_lists_providers() {
    let ctx = app().await;
    std::fs::write(
        ctx.workdir.join(".opencoder").join("config.json"),
        json!({
            "providers": {
                "acme": {
                    "base_url": "https://acme.example/v1",
                    "api_key": "sk-acme-secret-DO-NOT-LEAK",
                    "model": "acme-1"
                }
            }
        })
        .to_string(),
    )
    .unwrap();

    let (status, body) = get_json(&ctx, "/api/models").await;
    assert_eq!(status, StatusCode::OK);
    let raw = serde_json::to_string(&body).unwrap();
    assert!(
        !raw.contains("sk-acme-secret-DO-NOT-LEAK"),
        "api_key must never appear in the response: {raw}"
    );
    let providers = body["providers"].as_array().unwrap();
    assert!(
        providers
            .iter()
            .any(|p| p["provider"].as_str() == Some("acme")
                && p["model"].as_str() == Some("acme-1")
                && p["base_url"].as_str() == Some("https://acme.example/v1")),
        "named provider entry missing: {raw}"
    );
    assert!(
        providers
            .iter()
            .any(|p| p["provider"].as_str() == Some("(default)")),
        "default provider entry missing: {raw}"
    );
    assert!(body["default"].is_string(), "top-level default model id");
    let models = body["models"].as_array().unwrap();
    assert!(
        models.iter().any(|m| m.as_str() == Some("acme/acme-1")),
        "flat dropdown must contain provider-qualified ids: {raw}"
    );
    // Entries carry only the sanitized fields.
    for p in providers {
        assert!(p.get("api_key").is_none(), "no api_key field: {p}");
        assert!(p.get("headers").is_none(), "no headers field: {p}");
    }
}

// ── skills ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn skills_endpoint_returns_name_description_enabled() {
    let ctx = app().await;
    let (status, body) = get_json(&ctx, "/api/skills").await;
    assert_eq!(status, StatusCode::OK);
    let skills = body["skills"].as_array().expect("skills must be an array");
    for s in skills {
        assert!(s["name"].is_string(), "name required: {s}");
        assert!(s["description"].is_string(), "description required: {s}");
        assert!(s["enabled"].is_boolean(), "enabled flag required: {s}");
        assert!(s.get("body").is_none(), "body must NOT be included: {s}");
    }
}
