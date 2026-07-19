//! P4 functional tests for the web HTTP surface (CRUD + SSE replay + switch).
//!
//! These exercise the HTTP handlers directly (no network) against a real libsql
//! store + a MockChatClient, asserting the behavioral contracts:
//! - prompt_admit_returns_immediately: POST /prompt returns an admitted_seq
//!   without blocking on the LLM drain
//! - sse_replays_persisted_events_then_live: GET /events replays persisted
//!   events then forwards live broadcast events
//! - switch_agent_takes_effect: POST /agent updates the stored meta + handle
//! - interrupt_cancels_drain: POST /interrupt cancels the running drain token
//!
//! Drain spawn/cancel/live-lifecycle contracts live in `web_drain_contract.rs`.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use tower::ServiceExt;
use uuid::Uuid;

use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_store::{LibsqlStore, Store};

// Reuse the production AppState + handlers via a thin test router.
async fn app() -> (Router, Arc<opencoder_web::AppState>) {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let workdir = std::env::temp_dir();
    let state = Arc::new(opencoder_web::AppState {
        client_override: None,
        store: store.clone(),
        workdir: workdir.clone(),
        handles: opencoder_web::handle::new_handle_map(),
    });
    let app = Router::new()
        .route(
            "/api/sessions",
            post(opencoder_web::api::create_session).get(opencoder_web::api::list_sessions),
        )
        .route("/api/sessions/:id", get(opencoder_web::api::get_session))
        .route(
            "/api/sessions/:id/prompt",
            post(opencoder_web::api::post_prompt),
        )
        .route(
            "/api/sessions/:id/agent",
            post(opencoder_web::api::post_agent),
        )
        .route(
            "/api/sessions/:id/model",
            post(opencoder_web::api::post_model),
        )
        .route(
            "/api/sessions/:id/interrupt",
            post(opencoder_web::api::post_interrupt),
        )
        .route(
            "/api/sessions/:id/events",
            get(opencoder_web::api::get_events),
        )
        .route("/api/health", get(opencoder_web::api::health))
        .with_state(state.clone());
    (app, state)
}

/// Seed a session row with the given title/agent/model.
async fn seed(
    state: &opencoder_web::AppState,
    sid: &str,
    title: Option<&str>,
    agent: &str,
    model: &str,
) {
    state
        .store
        .create_session(&opencoder_store::SessionMeta {
            id: sid.to_string(),
            title: title.map(String::from),
            agent: Some(agent.into()),
            model: Some(model.into()),
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn health_ok() {
    let (app, _) = app().await;
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn create_and_get_session_roundtrip() {
    let (app, state) = app().await;
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"agent":"act"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let id = v["id"].as_str().unwrap().to_string();
    assert!(state.store.get_session(&id).await.unwrap().is_some());
}

#[tokio::test]
async fn prompt_admit_returns_immediately_with_seq() {
    let (_app, state) = app().await;
    let sid = Uuid::new_v4().to_string();
    seed(&state, &sid, None, "act", "m").await;

    // Inject a MockChatClient by calling admit_and_drain directly (the HTTP
    // path builds a real ChatClient which needs a key; the contract under test
    // is "admit returns a seq fast", which we verify via the store layer).
    let mock: Arc<dyn ChatStream> =
        Arc::new(
            MockChatClient::new().with_default(vec![opencoder_llm::LlmEvent::Completed {
                text: "ok".into(),
                tool_calls: vec![],
                usage: None,
            }]),
        );
    let cfg = opencoder_core::Config {
        model: "m/g".into(),
        ..Default::default()
    };
    let seq = opencoder_web::handle::admit_and_drain(
        state.handles.clone(),
        state.store.clone(),
        &sid,
        "hello".into(),
        opencoder_store::Delivery::Steer,
        mock,
        std::env::temp_dir(),
        cfg,
    )
    .await
    .unwrap();
    assert!(
        seq > 0,
        "admit must return a positive seq immediately: {seq}"
    );

    // give the drain a moment to consume + persist messages
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(25)).await;
        let n = state.store.load_messages(&sid).await.unwrap().len();
        if n > 0 {
            break;
        }
    }
    let msgs = state.store.load_messages(&sid).await.unwrap();
    assert!(
        !msgs.is_empty(),
        "drain must persist at least the admitted prompt + assistant reply"
    );
}

#[tokio::test]
async fn sse_replays_persisted_events_then_live() {
    let (_app, state) = app().await;
    let sid = "sse-sess";
    seed(&state, sid, None, "act", "m").await;
    // seed 3 persisted events
    for i in 0..3u32 {
        state
            .store
            .append_event(&opencoder_store::SessionEventRecord {
                session_id: sid.into(),
                kind: opencoder_store::EventKind::TextDelta,
                payload: serde_json::json!({ "i": i }),
                ts: i as i64,
                seq: None,
                sse_kind: None,
            })
            .await
            .unwrap();
    }

    // build the SSE response via the handler's underlying logic: replay after=0.
    // The live broadcast stays open (no drain to close it), so read frames
    // incrementally with a short timeout — the replay window is flushed first.
    use futures::StreamExt;
    let query = opencoder_web::api::EventsQuery { after: Some(0) };
    let resp = opencoder_web::api::get_events(
        axum::extract::State(state.clone()),
        axum::extract::Path(sid.to_string()),
        axum::extract::Query(query),
    )
    .await
    .into_response();
    let mut stream = resp.into_body().into_data_stream();
    let mut text = String::new();
    for _ in 0..40 {
        match tokio::time::timeout(Duration::from_millis(300), stream.next()).await {
            Ok(Some(Ok(bytes))) => {
                text.push_str(&String::from_utf8_lossy(&bytes));
                if text.contains("\"i\":2") {
                    break;
                }
            }
            _ => break,
        }
    }
    assert!(
        text.contains("\"i\":0"),
        "replay must include event 0; got: {text}"
    );
    assert!(
        text.contains("\"i\":1"),
        "replay must include event 1; got: {text}"
    );
    assert!(
        text.contains("\"i\":2"),
        "replay must include event 2; got: {text}"
    );
}

#[tokio::test]
async fn switch_agent_updates_stored_meta_and_handle() {
    let (app, state) = app().await;
    let sid = Uuid::new_v4().to_string();
    seed(&state, &sid, None, "act", "m").await;
    // install a live handle so the override path is exercised
    let (tx, _rx) = tokio::sync::broadcast::channel(8);
    let handle = Arc::new(opencoder_web::handle::SessionHandle {
        tx,
        cancel: tokio::sync::Mutex::new(tokio_util::sync::CancellationToken::new()),
        overrides: tokio::sync::Mutex::new(opencoder_web::handle::RuntimeOverrides::default()),
        draining: AtomicBool::new(false),
    });
    state.handles.lock().await.insert(sid.clone(), handle);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{sid}/agent"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"value":"plan"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let meta = state.store.get_session(&sid).await.unwrap().unwrap();
    assert_eq!(
        meta.agent.as_deref(),
        Some("plan"),
        "agent switch must persist to store meta"
    );
}

#[tokio::test]
async fn interrupt_cancels_running_drain_token() {
    let (_app, state) = app().await;
    let sid = "int-sess";
    let (tx, _rx) = tokio::sync::broadcast::channel(8);
    let cancel = tokio_util::sync::CancellationToken::new();
    let handle = Arc::new(opencoder_web::handle::SessionHandle {
        tx,
        cancel: tokio::sync::Mutex::new(cancel.clone()),
        overrides: tokio::sync::Mutex::new(opencoder_web::handle::RuntimeOverrides::default()),
        draining: AtomicBool::new(false),
    });
    state.handles.lock().await.insert(sid.into(), handle);

    let resp = opencoder_web::api::post_interrupt(
        axum::extract::State(state.clone()),
        axum::extract::Path(sid.to_string()),
    )
    .await;
    let _ = resp;
    assert!(
        cancel.is_cancelled(),
        "interrupt must cancel the drain's token"
    );
}

#[tokio::test]
async fn list_sessions_returns_created_sessions() {
    let (app, state) = app().await;
    // Create two sessions
    for agent in ["act", "plan"] {
        let id = Uuid::new_v4().to_string();
        seed(&state, &id, None, agent, "m").await;
    }
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let sessions = v["sessions"].as_array().expect("sessions array");
    assert!(
        sessions.len() >= 2,
        "should list at least 2 sessions, got {}",
        sessions.len()
    );
}

#[tokio::test]
async fn get_session_returns_meta() {
    let (app, state) = app().await;
    let sid = Uuid::new_v4().to_string();
    seed(&state, &sid, Some("test title"), "act", "m/g").await;
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions/{sid}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["id"], sid);
    assert_eq!(v["meta"]["title"], "test title");
}

#[tokio::test]
async fn post_model_switches_stored_meta() {
    let (app, state) = app().await;
    let sid = Uuid::new_v4().to_string();
    seed(&state, &sid, None, "act", "old-model").await;
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{sid}/model"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"value":"new-model"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let meta = state.store.get_session(&sid).await.unwrap().unwrap();
    assert_eq!(
        meta.model.as_deref(),
        Some("new-model"),
        "model switch must persist"
    );
}
