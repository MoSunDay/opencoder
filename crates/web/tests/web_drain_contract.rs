//! Drain-lifecycle + HTTP edge/error contracts for the web layer.
//!
//! These target the background drain spawn/cancel/SSE-live semantics and the
//! non-happy-path HTTP behavior, all driven through real handlers/fns against
//! an in-memory store + MockChatClient (no network). They close the
//! verification gaps around the F1 fix (drain presence ≠ running):
//! - pre_existing_events_handle_does_not_block_drain: a handle created by an
//!   early /events subscriber does not prevent a drain from spawning.
//! - second_prompt_after_drain_completion_spawns_fresh_drain: the `draining`
//!   flag is reset on completion (DrainGuard), so a follow-up prompt re-spawns.
//! - prompt_after_interrupt_runs_to_completion: the per-spawn cancel-token
//!   refresh means a prior interrupt cannot permanently poison a future drain.
//! - events_subscriber_before_prompt_receives_live: an early /events subscriber
//!   shares the broadcast channel and receives the live drain output.
//! - post_prompt_returns_500_on_malformed_config: POST /prompt surfaces config
//!   load failures as a structured 500 (config/api_key/client error path).
//! - events_stream_survives_subscriber_lag: GET /events keeps delivering to a
//!   slow subscriber (broadcast lag is dropped, never blocks the runner).

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use axum::response::IntoResponse;
use futures::StreamExt;
use opencoder_core::ContentBlock;
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{LibsqlStore, Store};
use serde_json::json;

/// Fresh in-memory AppState (drain tests call handlers/fns directly, no router).
async fn state() -> Arc<opencoder_web::AppState> {
    state_with_workdir(std::env::temp_dir()).await
}

/// AppState backed by an in-memory store but a custom workdir (for tests that
/// need to place config files on disk).
async fn state_with_workdir(workdir: std::path::PathBuf) -> Arc<opencoder_web::AppState> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    Arc::new(opencoder_web::AppState {
        store,
        workdir,
        handles: opencoder_web::handle::new_handle_map(),
    })
}

/// Seed a session row (default agent "act", model "m").
async fn seed(state: &opencoder_web::AppState, sid: &str) {
    state
        .store
        .create_session(&opencoder_store::SessionMeta {
            id: sid.to_string(),
            title: None,
            agent: Some("act".into()),
            model: Some("m".into()),
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
        })
        .await
        .unwrap();
}

/// Mock that completes a single assistant turn replying `text`.
fn mock_reply(text: &str) -> Arc<dyn ChatStream> {
    Arc::new(
        MockChatClient::new().with_default(vec![LlmEvent::Completed {
            text: text.into(),
            tool_calls: vec![],
            usage: None,
        }]),
    )
}

/// Admit a prompt and spawn its drain, returning the admitted seq. Wraps the
/// production `admit_and_drain` so each test stays focused on the contract.
async fn admit(state: &opencoder_web::AppState, sid: &str, prompt: &str, reply: &str) -> i64 {
    opencoder_web::handle::admit_and_drain(
        state.handles.clone(),
        state.store.clone(),
        sid,
        prompt.to_string(),
        opencoder_store::Delivery::Steer,
        mock_reply(reply),
        std::env::temp_dir(),
        opencoder_core::Config {
            model: "m/g".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap()
}

/// True once an assistant Text block containing `needle` is persisted.
async fn replied(state: &opencoder_web::AppState, sid: &str, needle: &str) -> bool {
    state
        .store
        .load_messages(sid)
        .await
        .unwrap()
        .iter()
        .flat_map(|m| m.blocks.iter())
        .any(|b| matches!(b, ContentBlock::Text { text } if text.contains(needle)))
}

/// Poll until `replied` holds or ~3s elapse.
async fn eventually_replied(state: &opencoder_web::AppState, sid: &str, needle: &str) -> bool {
    for _ in 0..120 {
        if replied(state, sid, needle).await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    false
}

/// Poll until the session's drain is idle (`draining` reset). The handle stays
/// in the map after completion, so this also asserts the DrainGuard ran.
async fn wait_idle(state: &opencoder_web::AppState, sid: &str) {
    for _ in 0..120 {
        let idle = state
            .handles
            .lock()
            .await
            .get(sid)
            .map(|h| !h.draining.load(Ordering::SeqCst))
            .unwrap_or(true);
        if idle {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("drain for {sid} never went idle");
}

/// Regression: a `SessionHandle` pre-existing in the map (e.g. created by an
/// early GET /events subscriber, with no drain running) must NOT prevent
/// admit_and_drain from spawning a drain — otherwise the prompt is admitted but
/// never processed. The `draining` flag, not map presence, gates spawning.
#[tokio::test]
async fn pre_existing_events_handle_does_not_block_drain() {
    let state = state().await;
    let sid = "pre";
    seed(&state, sid).await;
    // Simulate an early SSE subscriber: a handle sits in the map with no drain.
    state
        .handles
        .lock()
        .await
        .insert(sid.to_string(), opencoder_web::handle::SessionHandle::new());

    admit(&state, sid, "hello", "ok").await;
    assert!(
        eventually_replied(&state, sid, "ok").await,
        "pre-existing events handle must not swallow the prompt"
    );
}

/// G1: after a drain completes, the `draining` flag is reset (DrainGuard) and
/// the handle is reused, so a second prompt spawns a FRESH drain. If the guard
/// failed to reset, the CAS would see draining=true and never spawn, leaving
/// the second reply unpersisted.
#[tokio::test]
async fn second_prompt_after_drain_completion_spawns_fresh_drain() {
    let state = state().await;
    let sid = "g1";
    seed(&state, sid).await;

    admit(&state, sid, "first", "first-reply").await;
    wait_idle(&state, sid).await;
    assert!(
        eventually_replied(&state, sid, "first-reply").await,
        "first drain must persist its reply"
    );
    let after_first = state.store.load_messages(sid).await.unwrap().len();

    admit(&state, sid, "second", "second-reply").await;
    wait_idle(&state, sid).await;
    assert!(
        eventually_replied(&state, sid, "second-reply").await,
        "second prompt must spawn a fresh drain (draining was reset)"
    );
    let after_second = state.store.load_messages(sid).await.unwrap().len();
    assert!(
        after_second > after_first,
        "second drain must add messages ({after_second} <= {after_first})"
    );
}

/// G2: a prior interrupt cancels the handle's token permanently, but each spawn
/// refreshes a fresh token. A follow-up prompt must therefore run to completion.
/// If the stale cancelled token were reused, run_loop would abort at its
/// top-of-loop cancel checkpoint before any LLM call and never persist a reply.
#[tokio::test]
async fn prompt_after_interrupt_runs_to_completion() {
    let state = state().await;
    let sid = "g2";
    seed(&state, sid).await;

    admit(&state, sid, "first", "first-reply").await;
    wait_idle(&state, sid).await;

    // Cancel the (now-idle) token, simulating a prior interrupt's permanent
    // cancellation lingering on the reused handle.
    let h = state
        .handles
        .lock()
        .await
        .get(sid)
        .cloned()
        .expect("handle persists after drain");
    h.cancel.lock().await.cancel();
    assert!(h.cancel.lock().await.is_cancelled());

    admit(&state, sid, "second", "second-reply").await;
    assert!(
        eventually_replied(&state, sid, "second-reply").await,
        "post-interrupt prompt must run to completion (cancel token refreshed)"
    );
}

/// G3: a client subscribing to /events BEFORE the first prompt creates a handle
/// with no drain. The subsequent prompt must (a) still spawn a drain and (b)
/// broadcast live events to that early subscriber over the shared channel.
#[tokio::test]
async fn events_subscriber_before_prompt_receives_live() {
    let state = state().await;
    let sid = "g3";
    seed(&state, sid).await;

    // 1. subscribe first — get_events creates a handle with no drain.
    let resp = opencoder_web::api::get_events(
        axum::extract::State(state.clone()),
        axum::extract::Path(sid.to_string()),
        axum::extract::Query(opencoder_web::api::EventsQuery { after: Some(0) }),
    )
    .await
    .into_response();
    let mut stream = resp.into_body().into_data_stream();

    // 2. admit a prompt whose drain streams a live text_delta.
    let mock: Arc<dyn ChatStream> = Arc::new(MockChatClient::new().with_default(vec![
        LlmEvent::TextDelta("hello-live".into()),
        LlmEvent::Completed {
            text: "hello-live".into(),
            tool_calls: vec![],
            usage: None,
        },
    ]));
    opencoder_web::handle::admit_and_drain(
        state.handles.clone(),
        state.store.clone(),
        sid,
        "ping".into(),
        opencoder_store::Delivery::Steer,
        mock,
        std::env::temp_dir(),
        opencoder_core::Config {
            model: "m/g".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    // 3. the early subscriber must receive the live broadcast.
    let mut text = String::new();
    for _ in 0..120 {
        if let Ok(Some(Ok(bytes))) =
            tokio::time::timeout(Duration::from_millis(50), stream.next()).await
        {
            text.push_str(&String::from_utf8_lossy(&bytes));
            if text.contains("hello-live") {
                break;
            }
        }
    }
    assert!(
        text.contains("hello-live"),
        "early /events subscriber must receive the live drain broadcast; got: {text}"
    );
}

/// POST /prompt must surface a config-load failure as a structured 500 instead
/// of admitting a prompt it cannot run. Uses a malformed opencoder.json so the
/// error is deterministic and independent of any ambient API key env var.
#[tokio::test]
async fn post_prompt_returns_500_on_malformed_config() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("opencoder.json"), "{ not valid json").unwrap();
    let state = state_with_workdir(dir.path().to_path_buf()).await;

    let resp = opencoder_web::api::post_prompt(
        axum::extract::State(state),
        axum::extract::Path("any-sid".to_string()),
        axum::extract::Json(opencoder_web::api::PromptBody {
            prompt: "hi".into(),
            delivery: None,
            agent: None,
            model: None,
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["ok"], false, "error body must signal ok:false");
    assert!(
        v["error"].as_str().unwrap_or_default().contains("config"),
        "error must mention config: {}",
        v["error"]
    );
}

/// GET /events must keep delivering to a slow subscriber: when the drain
/// out-paces the receiver, broadcast lag is dropped by the `.ok()` filter
/// (never propagated as a stream error) and recent events still arrive. The
/// runner's `let _ = tx.send(..)` is inherently non-blocking; this asserts the
/// SSE consumer side degrades gracefully.
#[tokio::test]
async fn events_stream_survives_subscriber_lag() {
    let state = state().await;
    let sid = "lag";
    seed(&state, sid).await;

    // subscribe first — get_events creates the handle and hands back a receiver.
    let resp = opencoder_web::api::get_events(
        axum::extract::State(state.clone()),
        axum::extract::Path(sid.to_string()),
        axum::extract::Query(opencoder_web::api::EventsQuery { after: Some(0) }),
    )
    .await
    .into_response();
    let mut stream = resp.into_body().into_data_stream();

    // push far beyond the broadcast capacity (256) directly via the handle's tx,
    // simulating a drain that has out-run a subscriber not yet reading frames.
    let tx = state.handles.lock().await.get(sid).unwrap().tx.clone();
    for i in 0..600u32 {
        let _ = tx.send(opencoder_web::handle::SseEvt {
            kind: "text_delta".into(),
            data: json!({ "i": i }),
            ts: i as i64,
            seq: None,
        });
    }

    // consume — the `.ok()` filter must swallow the Lagged error; the stream
    // keeps the most recent window and never panics or deadlocks.
    let mut text = String::new();
    for _ in 0..400 {
        if let Ok(Some(Ok(bytes))) =
            tokio::time::timeout(Duration::from_millis(50), stream.next()).await
        {
            text.push_str(&String::from_utf8_lossy(&bytes));
            if text.contains("\"i\":599") {
                break;
            }
        }
    }
    assert!(
        text.contains("\"i\":599"),
        "lagged subscriber must still receive the most recent event; got: {text}"
    );
}
