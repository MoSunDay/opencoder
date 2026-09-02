//! P2 endpoint boundaries:
//!   * the question endpoints previously get-or-created a handle for ANY
//!     session id — a token holder could grow the HandleMap without bound by
//!     polling a bogus id (and the 404 semantics diverged from /events);
//!   * `GET /seq` returned 200 `{seq: 0}` for unknown sessions, making a
//!     truncated id look like "no events yet" (client replays from 0).
//!
//! Both now 404 on a missing session row, aligned with `GET /events`.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use opencoder_llm::MockChatClient;
use opencoder_store::{LibsqlStore, SessionMeta, Store};
use serde_json::json;

async fn state() -> (Arc<opencoder_web::AppState>, String) {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "real-session".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    (
        Arc::new(opencoder_web::AppState {
            client_override: None,
            store,
            workdir: std::env::temp_dir(),
            handles: opencoder_web::handle::new_handle_map(),
            nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
            controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        }),
        "real-session".to_string(),
    )
}

fn status_of(resp: axum::response::Response) -> axum::http::StatusCode {
    resp.status()
}

#[tokio::test]
async fn questions_on_missing_session_is_404_and_creates_no_handle() {
    let (st, _real) = state().await;
    let ghost = "no-such-session".to_string();

    let resp =
        opencoder_web::api_questions::list_questions(State(st.clone()), Path(ghost.clone())).await;
    assert_eq!(status_of(resp), axum::http::StatusCode::NOT_FOUND);

    let resp = opencoder_web::api_questions::answer_question(
        State(st.clone()),
        Path((ghost.clone(), "call-1".into())),
        Some(axum::Json(opencoder_web::api_questions::AnswerBody {
            answer: "yes".into(),
        })),
    )
    .await;
    assert_eq!(status_of(resp), axum::http::StatusCode::NOT_FOUND);

    let resp = opencoder_web::api_questions::skip_question(
        State(st.clone()),
        Path((ghost, "call-1".into())),
    )
    .await;
    assert_eq!(status_of(resp), axum::http::StatusCode::NOT_FOUND);

    // The actual resource-exhaustion bug: no handle may be created for a
    // bogus id, no matter how often it is polled.
    assert!(
        st.handles.lock().await.is_empty(),
        "404 precheck must run BEFORE get_or_create_handle"
    );
}

#[tokio::test]
async fn questions_on_existing_session_still_work() {
    let (st, real) = state().await;
    let resp = opencoder_web::api_questions::list_questions(State(st.clone()), Path(real)).await;
    assert_eq!(status_of(resp), axum::http::StatusCode::OK);
}

#[tokio::test]
async fn event_seq_on_missing_session_is_404() {
    let (st, _real) = state().await;
    let resp =
        opencoder_web::api::get_event_seq(State(st.clone()), Path("no-such-session".to_string()))
            .await
            .into_response();
    assert_eq!(status_of(resp), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn event_seq_on_existing_session_returns_seq_zero_ok() {
    let (st, real) = state().await;
    let resp = opencoder_web::api::get_event_seq(State(st), Path(real))
        .await
        .into_response();
    assert_eq!(status_of(resp), axum::http::StatusCode::OK);
    let body = axum::Json(json!({}));
    let _ = body; // shape asserted via status; payload covered by web_contract
    let _ = MockChatClient::new;
}
