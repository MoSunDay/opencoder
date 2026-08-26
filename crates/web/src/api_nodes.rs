//! Node registry HTTP surface (`/api/nodes*`, registry + dispatch half).
//!
//! Split from `api.rs` for the file-size budget. Task *operations*
//! (claim / upload / status / cancel) live in `api_nodes_ops.rs`; the browser
//! SSE stream lives in `sse_nodes.rs`. Handlers are pure composition over the
//! [`Store`] node API — no business logic here.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use opencoder_core::node_protocol::{
    NodeDispatchRequest, NodeDispatchResponse, NodeHeartbeatResponse, NodeRegisterRequest,
    NodeRegisterResponse,
};
use opencoder_store::EventKind;

use crate::api::{error_400, error_404, error_500};
use crate::api_nodes_ops::{emit_closure, ClosureEvent};
use crate::nodes_state::{compute_status, STALE_AFTER_MS};
use crate::AppState;

/// GET /api/nodes — fleet registry view.
///
/// Read path IS maintenance (opportunistic sweep): before listing, tasks of
/// nodes whose heartbeat went silent beyond [`STALE_AFTER_MS`] while
/// `running|cancelling` are converged to a terminal `error("node lost")` via
/// `converge_lost_node_tasks` — there is no background sweeper, so this
/// endpoint doubles as the reaper.
///
/// Known trade-off on network partitions with double execution: if the worker
/// is actually alive and uploads late, its terminal report hits an
/// already-frozen task and is rejected by the store's transition guard;
/// surfacing `error(node lost)` here is the committed behavior, not a bug.
pub async fn list_nodes(State(state): State<Arc<AppState>>) -> Response {
    let now = chrono::Utc::now().timestamp_millis();
    // Converge BEFORE assembling the response body so statuses below already
    // reflect post-sweep reality (freed busy bits → idle rows), not a lagging
    // pre-sweep snapshot.
    let swept = match state
        .store
        .converge_lost_node_tasks(now, STALE_AFTER_MS)
        .await
    {
        Ok(records) => records,
        Err(e) => return error_500(format!("converge_lost_node_tasks: {e:#}")),
    };
    for r in &swept {
        let closure = ClosureEvent {
            session_id: &r.session_id,
            kind: EventKind::Error,
            sse_kind: "error",
            task_id: &r.id,
            ok: false,
            error: Some("node lost"),
            cancel: false,
        };
        if let Err(e) = emit_closure(&state, closure, now).await {
            return error_500(e);
        }
    }
    // Fresh DB rows carry raw statuses written by store transitions
    // (`online`/`idle`/`busy`); liveness is layered on top per-request via
    // [`compute_status`], so a missing heartbeat crosses into `lost`.
    match state.store.list_nodes().await {
        Ok(nodes) => {
            let nodes: Vec<serde_json::Value> = nodes
                .iter()
                .map(|n| {
                    json!({
                        "id": n.id,
                        "name": n.name,
                        "version": n.version,
                        "workdir": n.workdir,
                        "first_seen": n.first_seen,
                        "last_seen_at": n.last_seen_at,
                        // Computed, not the stored raw string.
                        "status": compute_status(n.last_seen_at, &n.last_status, now),
                        "last_task_id": n.last_task_id,
                    })
                })
                .collect();
            Json(json!({ "nodes": nodes })).into_response()
        }
        Err(e) => error_500(format!("list_nodes: {e:#}")),
    }
}

/// POST /api/nodes/register — idempotent by `name`.
pub async fn post_register(
    State(state): State<Arc<AppState>>,
    Json(body): Json<NodeRegisterRequest>,
) -> Response {
    if body.name.trim().is_empty() {
        return error_400("name must not be empty".into());
    }
    let now = chrono::Utc::now().timestamp_millis();
    match state
        .store
        .register_node(
            &body.name,
            body.version.as_deref(),
            body.workdir.as_deref(),
            now,
        )
        .await
    {
        Ok(rec) => Json(NodeRegisterResponse { node_id: rec.id }).into_response(),
        Err(e) => error_500(format!("register_node: {e:#}")),
    }
}

/// POST /api/nodes/:id/heartbeat — touch liveness and collect cancel commands.
pub async fn post_heartbeat(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    // Unknown node → 404 (store::heartbeat_node would bail with a message we'd
    // otherwise have to string-match; an explicit pre-check keeps it typed).
    match state.store.get_node(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return error_404("node not found"),
        Err(e) => return error_500(format!("get_node: {e:#}")),
    }
    let now = chrono::Utc::now().timestamp_millis();
    match state.store.heartbeat_node(&id, now).await {
        Ok(cancel_task_ids) => Json(NodeHeartbeatResponse {
            server_time_ms: now,
            cancel_task_ids,
        })
        .into_response(),
        Err(e) => error_500(format!("heartbeat_node: {e:#}")),
    }
}

/// DELETE /api/nodes/:id — remove node + its queue + synthetic sessions.
pub async fn delete_node(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.store.delete_node(&id).await {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => error_500(format!("delete_node: {e:#}")),
    }
}

/// GET /api/nodes/:id/tasks — the dispatch queue as seen by UIs (newest last).
pub async fn list_tasks(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.store.list_node_tasks(&id, 200).await {
        Ok(tasks) => Json(json!({ "tasks": tasks })).into_response(),
        Err(e) => error_500(format!("list_node_tasks: {e:#}")),
    }
}

/// POST /api/nodes/:id/tasks — enqueue one task with its synthetic session.
///
/// Both ids are fresh ULIDs (`opencode_session::runner::new_id`); the session
/// row is created inside the store's dispatch transaction with
/// `task_type="node"`, which is what hides it from normal session listings.
pub async fn dispatch_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<NodeDispatchRequest>,
) -> Response {
    if body.prompt.trim().is_empty() {
        return error_400("prompt must not be empty".into());
    }
    match state.store.get_node(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return error_404("node not found"),
        Err(e) => return error_500(format!("get_node: {e:#}")),
    }
    let task_id = opencoder_session::runner::new_id();
    let session_id = opencoder_session::runner::new_id();
    let now = chrono::Utc::now().timestamp_millis();
    match state
        .store
        .dispatch_node_task(
            &task_id,
            &session_id,
            &id,
            body.title.as_deref(),
            &body.prompt,
            body.agent.as_deref(),
            body.model.as_deref(),
            now,
        )
        .await
    {
        Ok(rec) => Json(NodeDispatchResponse {
            task_id: rec.id,
            session_id: rec.session_id,
        })
        .into_response(),
        Err(e) => error_500(format!("dispatch_node_task: {e:#}")),
    }
}
