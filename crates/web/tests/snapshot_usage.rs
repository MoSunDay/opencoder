//! Contract test for the session snapshot's lifetime `usage` aggregate
//! (`GET /api/sessions/:id`): parent rounds from bare `llm_usage` rows plus
//! child rounds from wrapped `subagent_child(llm_usage)` rows — including
//! local-memory maintenance, whose spend never lands on a message row. The
//! SPA footer reloads from this field; `null` keeps the client-side
//! per-message fallback.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use opencoder_session::SessionEvent;
use opencoder_store::{EventKind, LibsqlStore, SessionEventRecord, SessionMeta, Store};

async fn app() -> (axum::Router, Arc<opencoder_web::AppState>) {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let state = Arc::new(opencoder_web::AppState {
        config_home: None,
        client_override: None,
        brain: opencoder_web::api_brain::mock_brain(store.clone()),
        store: store.clone(),
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        team: opencoder_web::team_state::mock(),
        project: opencoder_web::ProjectService::new(),
    });
    (opencoder_web::build_app(state.clone(), None, false), state)
}

async fn seed_session(state: &opencoder_web::AppState, sid: &str) {
    state
        .store
        .create_session(&SessionMeta {
            id: sid.to_string(),
            title: None,
            agent: Some("act".into()),
            model: None,
            autopilot_mode: None,
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
            kind: None,
        })
        .await
        .unwrap();
}

fn sse_row(sid: &str, ev: &SessionEvent, ts: i64) -> SessionEventRecord {
    SessionEventRecord {
        session_id: sid.into(),
        kind: EventKind::Step,
        payload: ev.sse_data(),
        ts,
        seq: None,
        sse_kind: Some(ev.sse_kind().to_string()),
    }
}

async fn get_json(router: &axum::Router, sid: &str) -> serde_json::Value {
    let res = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions/{sid}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn snapshot_usage_includes_child_and_memory_spend() {
    let (router, state) = app().await;
    seed_session(&state, "p").await;
    let mut a1 = opencoder_core::Message::assistant("a1");
    a1.usage.total_tokens = 1_000_000;
    state.store.append_messages("p", &[a1]).await.unwrap();
    // Parent round (bare) + maintenance round (wrapped), both in the sink's
    // SSE payload form.
    let rows = vec![
        sse_row(
            "p",
            &SessionEvent::LlmUsage {
                total_tokens: 1_000_000,
                input_tokens: 900_000,
                output_tokens: 100_000,
            },
            1,
        ),
        sse_row(
            "p",
            &SessionEvent::SubagentChild {
                id: "memory-1".into(),
                ev: Box::new(SessionEvent::LlmUsage {
                    total_tokens: 1234,
                    input_tokens: 1000,
                    output_tokens: 234,
                }),
            },
            2,
        ),
    ];
    state.store.append_events(&rows).await.unwrap();

    let body = get_json(&router, "p").await;
    assert_eq!(body["usage"]["total_tokens"], 1_001_234);
    assert_eq!(body["usage"]["input_tokens"], 901_000);
    assert_eq!(body["usage"]["output_tokens"], 100_234);
}

#[tokio::test]
async fn snapshot_usage_is_null_without_usage_events() {
    let (router, state) = app().await;
    seed_session(&state, "quiet").await;
    let mut a1 = opencoder_core::Message::assistant("a1");
    a1.usage.total_tokens = 7;
    state.store.append_messages("quiet", &[a1]).await.unwrap();
    let body = get_json(&router, "quiet").await;
    // No event-log usage → null, so the SPA keeps its message-sum fallback.
    assert!(body["usage"].is_null());
    assert_eq!(body["messages"][0]["usage"]["total_tokens"], 7);
}
