//! `POST /sessions/:id/prompt` used to silently degrade an invalid `delivery`
//! (e.g. a "stear" typo) to `Steer`, interrupting the running turn. The
//! handler now rejects a present-but-unparseable value with a structured 400;
//! a missing field keeps the `Steer` default. `Delivery::parse` trims, so a
//! padded `" queue "` is accepted (previously it fell back to `Steer` too).
//!
//! Handler-level tests (no router): an injected MockChatClient gets the
//! handler deterministically past client construction without any ambient
//! config or API key.

use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::IntoResponse;
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{Delivery, LibsqlStore, Store};

/// Fresh state: in-memory store, injected mock client, and an ISOLATED
/// tempdir workdir + pinned model so the drain never depends on ambient
/// config files or env keys (mirrors web_drain_contract / store_error_surfacing).
async fn state() -> Arc<opencoder_web::AppState> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock: Arc<dyn ChatStream> =
        Arc::new(
            MockChatClient::new().with_default(vec![LlmEvent::Completed {
                text: "ok".into(),
                tool_calls: vec![],
                usage: None,
            }]),
        );
    Arc::new(opencoder_web::AppState {
        client_override: Some(mock),
        store,
        workdir: tempfile::tempdir().unwrap().keep(),
        handles: opencoder_web::handle::new_handle_map(),
    })
}

/// Call `post_prompt` with the given `delivery` and return (status, body).
async fn post(
    state: &Arc<opencoder_web::AppState>,
    sid: &str,
    delivery: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let resp = opencoder_web::api::post_prompt(
        axum::extract::State(state.clone()),
        axum::extract::Path(sid.to_string()),
        axum::Json(opencoder_web::api::PromptBody {
            prompt: "hi".into(),
            images: Vec::new(),
            delivery: delivery.map(String::from),
            agent: None,
            model: Some("m/g".into()),
            skill: None,
        }),
    )
    .await
    .into_response();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    (status, v)
}

/// A padded `" queue "` must be admitted (200) — not silently degraded to
/// `Steer` by the old untrimmed parse. Bounded-poll until the drain persists
/// the turn, mirroring web_contract's admit test.
#[tokio::test]
async fn padded_queue_delivery_is_admitted() {
    let state = state().await;
    let (status, v) = post(&state, "s-pad", Some(" queue ")).await;
    assert_eq!(status, StatusCode::OK, "body: {v}");
    assert_eq!(v["ok"], true);
    assert!(
        v["admitted_seq"].as_i64().unwrap_or(0) > 0,
        "must return a positive admitted seq: {v}"
    );
    // Bounded poll (~3s, mirroring web_drain_contract) until the drain
    // persists the turn; the budget survives parallel test-binary contention.
    for _ in 0..120 {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        if !state.store.load_messages("s-pad").await.unwrap().is_empty() {
            break;
        }
    }
    assert!(
        !state.store.load_messages("s-pad").await.unwrap().is_empty(),
        "drain must consume the queued prompt"
    );
}

/// An omitted `delivery` keeps the `Steer` default (still a 200).
#[tokio::test]
async fn missing_delivery_defaults_to_steer() {
    let state = state().await;
    let (status, v) = post(&state, "s-none", None).await;
    assert_eq!(status, StatusCode::OK, "body: {v}");
    assert_eq!(v["ok"], true);
}

/// A typo'd `delivery` must yield a structured 400 naming the valid values —
/// never a silent `Steer` fallback that interrupts the running turn.
#[tokio::test]
async fn invalid_delivery_is_a_400() {
    let state = state().await;
    let (status, v) = post(&state, "s-typo", Some("stear")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {v}");
    assert_eq!(v["ok"], false);
    let err = v["error"].as_str().unwrap_or_default();
    assert!(
        err.contains("delivery") && err.contains("steer") && err.contains("queue"),
        "error must name the field and its valid values: {err}"
    );
    // Nothing was admitted: no session row, no pending input of either kind.
    assert!(
        state.store.get_session("s-typo").await.unwrap().is_none(),
        "400 must happen before ensure_session_row"
    );
    assert!(state
        .store
        .pending_inputs("s-typo", Delivery::Steer)
        .await
        .unwrap()
        .is_empty());
    assert!(state
        .store
        .pending_inputs("s-typo", Delivery::Queue)
        .await
        .unwrap()
        .is_empty());
}

/// Whitespace-only and empty-string deliveries are also invalid (after trim).
#[tokio::test]
async fn blank_delivery_is_a_400() {
    let state = state().await;
    let (status, v) = post(&state, "s-blank", Some("   ")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {v}");
    assert_eq!(v["ok"], false);
}
