//! `GET /api/sessions/:id` snapshot assembly, split out of `api.rs` to
//! respect its file-size budget: meta + messages + run-state flags + the
//! lifetime usage aggregate the SPA footer reloads from.

use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::usage_totals::{session_usage, usage_json};
use crate::AppState;

/// The session snapshot body shared by `GET /sessions/:id` and
/// `GET /sessions/:id/messages`.
pub(crate) async fn messages_response(state: &AppState, id: &str) -> Response {
    let meta = match state.store.get_session(id).await {
        Ok(m) => m,
        Err(e) => return crate::api::error_500(format!("get_session: {e}")),
    };
    let messages = match state.store.load_messages(id).await {
        Ok(messages) => messages,
        Err(e) => return crate::api::error_500(format!("load_messages: {e}")),
    };
    // Run-state flag for the console: lets a client tell "stream live" from
    // "stream ended" without inferring from frames (found by real-browser
    // acceptance of the interrupt/reconnect flow).
    let draining = state
        .handles
        .lock()
        .await
        .get(id)
        .map(|h| h.draining.load(std::sync::atomic::Ordering::SeqCst))
        .unwrap_or(false);
    let harness = match state.store.harness_runtime(id).await {
        Ok(runtime) => runtime.unwrap_or_default().harness,
        Err(e) => return crate::api::error_500(format!("harness state: {e}")),
    };
    // Lifetime token totals incl. child spend (subagents + local-memory
    // maintenance) — the event log is the only reload source for the part
    // that never lands on message rows. Absent → null → the SPA falls back
    // to per-message sums.
    let usage = session_usage(&state.store, id).await;
    Json(json!({
        "id": id,
        "meta": meta,
        "harness": harness,
        "messages": messages,
        "draining": draining,
        "usage": usage_json(usage),
    }))
    .into_response()
}
