//! Functional tests for the new feature-parity endpoints:
//! fork, skill, config, compact, handoff, bg.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::{get, patch, post};
use axum::Router;
use tower::ServiceExt;

use opencoder_core::Message;
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_store::{LibsqlStore, Store};

fn app(state: Arc<opencoder_web::AppState>) -> Router {
    Router::new()
        .route(
            "/api/sessions",
            post(opencoder_web::api::create_session).get(opencoder_web::api::list_sessions),
        )
        .route(
            "/api/sessions/:id",
            get(opencoder_web::api::get_session).delete(opencoder_web::api::delete_session),
        )
        .route("/api/sessions/:id/fork", post(opencoder_web::api_ops::fork_session))
        .route("/api/sessions/:id/skill", post(opencoder_web::api_ops::post_skill))
        .route("/api/sessions/:id/compact", post(opencoder_web::api_ops::post_compact))
        .route("/api/sessions/:id/handoff", post(opencoder_web::api_ops::post_handoff))
        .route("/api/config", get(opencoder_web::api_ops::get_config))
        .route("/api/config", patch(opencoder_web::api_ops::patch_config))
        .route("/api/bg", get(opencoder_web::api_ops::list_bg))
        .route("/api/bg/stop", post(opencoder_web::api_ops::stop_bg))
        .route("/api/health", get(opencoder_web::api::health))
        .with_state(state)
}

async fn state() -> Arc<opencoder_web::AppState> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let workdir = std::env::temp_dir().join(format!("oc-web-ops-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workdir).ok();
    Arc::new(opencoder_web::AppState {
        client_override: Some(Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>),
        store,
        workdir,
        handles: opencoder_web::handle::new_handle_map(),
    })
}

async fn seed(state: &opencoder_web::AppState, sid: &str) {
    state.store.create_session(&opencoder_store::SessionMeta {
        id: sid.to_string(),
        title: Some("test".into()),
        agent: Some("act".into()),
        model: Some("m".into()),
        workdir_hash: None,
        created_at: 0,
        updated_at: 0,
        summary: None,
        summary_seq: None,
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
    }).await.unwrap();
}

fn assistant_with_text(id: &str, text: &str) -> Message {
    let mut m = Message::assistant(id.to_string());
    m.blocks.push(opencoder_core::ContentBlock::text(text));
    m
}

#[tokio::test]
async fn fork_copies_messages_and_returns_new_id() {
    let state = state().await;
    let app = app(state.clone());
    seed(&state, "parent").await;
    state.store.append_messages("parent", &[
        Message::user("u1".to_string(), "hello"),
        assistant_with_text("a1", "hi there"),
    ]).await.unwrap();

    let resp = app.oneshot(Request::builder()
        .method("POST").uri("/api/sessions/parent/fork")
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let new_id = v["id"].as_str().expect("id");
    assert_ne!(new_id, "parent");

    let msgs = state.store.load_messages(new_id).await.unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].text(), "hello");
    assert_eq!(msgs[1].text(), "hi there");

    let parent = state.store.load_messages("parent").await.unwrap();
    assert_eq!(parent.len(), 2, "parent unchanged");
}

#[tokio::test]
async fn fork_nonexistent_returns_404() {
    let state = state().await;
    let app = app(state.clone());
    let resp = app.oneshot(Request::builder()
        .method("POST").uri("/api/sessions/nope/fork")
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn fork_title_gets_fork_suffix() {
    let state = state().await;
    let app = app(state.clone());
    seed(&state, "p2").await;
    let resp = app.oneshot(Request::builder()
        .method("POST").uri("/api/sessions/p2/fork")
        .body(Body::empty()).unwrap()).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let meta = state.store.get_session(v["id"].as_str().unwrap()).await.unwrap().unwrap();
    assert_eq!(meta.title.as_deref(), Some("test (fork)"));
}

#[tokio::test]
async fn skill_persists_to_store_meta() {
    let state = state().await;
    let app = app(state.clone());
    seed(&state, "s1").await;
    let resp = app.oneshot(Request::builder()
        .method("POST").uri("/api/sessions/s1/skill")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"skill":"repo-local-memory"}"#)).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let meta = state.store.get_session("s1").await.unwrap().unwrap();
    assert_eq!(meta.skill.as_deref(), Some("repo-local-memory"));
}

#[tokio::test]
async fn skill_clear_with_null() {
    let state = state().await;
    let app = app(state.clone());
    seed(&state, "s2").await;
    state.store.update_session("s2", &opencoder_store::SessionPatch {
        skill: Some("my-skill".into()), ..Default::default()
    }).await.unwrap();
    let resp = app.oneshot(Request::builder()
        .method("POST").uri("/api/sessions/s2/skill")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"skill":null}"#)).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let meta = state.store.get_session("s2").await.unwrap().unwrap();
    assert!(meta.skill.is_none(), "skill should be cleared");
}

#[tokio::test]
async fn get_config_returns_json() {
    let state = state().await;
    let app = app(state.clone());
    let resp = app.oneshot(Request::builder()
        .method("GET").uri("/api/config")
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(v.is_object());
    assert!(v.get("model").is_some());
}

#[tokio::test]
async fn patch_config_merges_and_persists() {
    let state = state().await;
    let app = app(state.clone());
    let resp = app.oneshot(Request::builder()
        .method("PATCH").uri("/api/config")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"model":"claude-test-model"}"#)).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let cfg = opencoder_core::Config::load(&state.workdir).unwrap();
    assert_eq!(cfg.model, "claude-test-model");
}

#[tokio::test]
async fn list_bg_returns_empty_array() {
    let state = state().await;
    let app = app(state.clone());
    let resp = app.oneshot(Request::builder()
        .method("GET").uri("/api/bg")
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(v["processes"].is_array());
}

#[tokio::test]
async fn stop_bg_returns_ok() {
    let state = state().await;
    let app = app(state.clone());
    let resp = app.oneshot(Request::builder()
        .method("POST").uri("/api/bg/stop")
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(v["ok"].as_bool().unwrap());
}

#[tokio::test]
async fn compact_returns_ok_and_queued() {
    let state = state().await;
    let app = app(state.clone());
    seed(&state, "c1").await;
    let resp = app.oneshot(Request::builder()
        .method("POST").uri("/api/sessions/c1/compact")
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn compact_nonexistent_returns_404() {
    let state = state().await;
    let app = app(state.clone());
    let resp = app.oneshot(Request::builder()
        .method("POST").uri("/api/sessions/nope/compact")
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn handoff_returns_ok_when_plan_exists() {
    let state = state().await;
    let app = app(state.clone());
    seed(&state, "h1").await;
    state.store.append_messages("h1", &[
        assistant_with_text("a1", "## Plan\n1. do X\n2. do Y"),
    ]).await.unwrap();
    let resp = app.oneshot(Request::builder()
        .method("POST").uri("/api/sessions/h1/handoff")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"extra":"begin"}"#)).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
}
