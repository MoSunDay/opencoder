//! Subagent observability + clear-all session management.
//!
//! - `GET /api/sessions/:id/subagents` lists the session's subagent task
//!   records — the durable source for `child_session_id` (the live SSE
//!   `subagent_start` event also carries it, but only while streaming). The
//!   SPA uses this both to drill into a child transcript and to restore
//!   subagent cards after a page refresh.
//! - `DELETE /api/sessions?keep=:id` clears every session except `keep`,
//!   mirroring the TUI `/task` clear-all: refused (409) while any live handle
//!   is draining, otherwise evicting non-keep live handles with the same
//!   teardown as `DELETE /api/sessions/:id` before the FK-cascading bulk
//!   delete.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use opencoder_store::SubagentTaskRecord;

use crate::AppState;

#[derive(Deserialize, Default)]
pub struct ClearQuery {
    /// The single session to keep; every other session row (and its
    /// cascading messages/inputs/events/subagent tasks) is deleted.
    pub keep: Option<String>,
}

/// GET /api/sessions/:id/subagents — the parent session's subagent tasks.
/// A present-but-empty list is a normal 200; only a missing parent is a 404.
pub async fn list_subagents(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    match state.store.get_session(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return error_404(&format!("session not found: {id}")),
        Err(e) => return error_500(format!("get_session: {e:#}")),
    }
    let tasks = match state.store.list_subagent_tasks(&id).await {
        Ok(t) => t,
        Err(e) => return error_500(format!("list_subagent_tasks: {e:#}")),
    };
    let items = tasks.iter().map(task_json).collect::<Vec<_>>();
    Json(json!({ "tasks": items })).into_response()
}

fn task_json(t: &SubagentTaskRecord) -> serde_json::Value {
    json!({
        "id": t.task_id,
        // `agent` in storage is the TUI card's `kind` (explore/build).
        "kind": t.agent,
        "status": t.status,
        "child_session_id": t.child_session_id,
        "prompt": t.prompt,
        "parent_message_id": t.parent_message_id,
        "result": t.result,
        "ok": t.ok,
        "created_at": t.started_at,
        "updated_at": t.completed_at.unwrap_or(t.started_at),
    })
}

/// DELETE /api/sessions?keep=:id — clear every session except `keep`.
///
/// Run gate mirrors the TUI `gate_clear_all`: any live draining handle (kept
/// session included — a running subagent's child session is still being
/// written to, and clearing would FK-violate its next append) refuses with
/// 409; retry at idle.
pub async fn clear_sessions(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ClearQuery>,
) -> Response {
    let keep = match q.keep {
        Some(ref k) if !k.is_empty() => k.clone(),
        _ => return error_400("missing ?keep=<session id>"),
    };
    match state.store.get_session(&keep).await {
        Ok(Some(_)) => {}
        Ok(None) => return error_404(&format!("session not found: {keep}")),
        Err(e) => return error_500(format!("get_session: {e:#}")),
    }
    if any_draining(&state).await {
        return error_409("clear refused while a session drain is running — retry when idle");
    }
    // Evict non-keep live handles with the same teardown as delete_session:
    // cancel the drain, fire child cancels, release waiting questions, then
    // clean up MCP connections (after dropping the map lock).
    let evicted: Vec<(String, Arc<crate::handle::SessionHandle>)> = {
        let mut map = state.handles.lock().await;
        let ids: Vec<String> = map.keys().filter(|k| **k != keep).cloned().collect();
        ids.into_iter()
            .filter_map(|id| map.remove(&id).map(|h| (id, h)))
            .collect()
    };
    for (id, h) in &evicted {
        h.cancel.lock().await.cancel();
        opencoder_session::fire_child_cancels(&h.child_cancels);
        crate::handle_questions::abandon_all_waiting(h);
        opencoder_session::mcp::cleanup(id).await;
    }
    let removed = match state.store.clear_other_sessions(&keep).await {
        Ok(n) => n,
        Err(e) => return error_500(format!("clear_other_sessions: {e:#}")),
    };
    Json(json!({ "ok": true, "removed": removed })).into_response()
}

/// True when any live handle is mid-drain. Checked under the map lock so the
/// gate sees a consistent snapshot (a drain started after the check gets a
/// fresh handle whose session row may already be gone — its next store write
/// fails loudly, same check-then-act window as the TUI gate).
async fn any_draining(state: &AppState) -> bool {
    let map = state.handles.lock().await;
    map.values()
        .any(|h| h.draining.load(Ordering::SeqCst))
}

// ── helpers (same shapes as the other api_* modules) ──────────────────────

fn error_400(msg: &str) -> Response {
    (
        axum::http::StatusCode::BAD_REQUEST,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

fn error_409(msg: &str) -> Response {
    (
        axum::http::StatusCode::CONFLICT,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

fn error_404(msg: &str) -> Response {
    (
        axum::http::StatusCode::NOT_FOUND,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

fn error_500(msg: String) -> Response {
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}
