//! Node registry HTTP surface (`/api/nodes*`, registry + dispatch half).
//!
//! Split from `api.rs` for the file-size budget. Task *operations*
//! (claim / upload / status / cancel) live in `api_nodes_ops.rs`; the browser
//! SSE stream lives in `sse_nodes.rs`. Handlers are pure composition over the
//! [`Store`] node API — no business logic here.

use std::sync::Arc;

use axum::extract::connect_info::ConnectInfo;
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
use crate::control_state::HEARTBEAT_CONTROL_BATCH;
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
    // Fan out one terminal error frame per converged task. The sweep is
    // already committed at this point, so a single fan-out failure must NOT
    // abort the loop (a `return 500` here would permanently drop the remaining
    // error frames and leave SSE clients of those sessions hanging): degrade
    // to a log line and keep going — the frames stay replayable from the store
    // once the transient write failure clears.
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
            tracing::warn!(
                task_id = %r.id,
                session_id = %r.session_id,
                error = %e,
                "lost-node sweep: failed to emit terminal error frame"
            );
        }
    }
    // Same opportunistic sweep for DAG runs: `running | cancelling` runs of
    // heartbeat-stale nodes converge to `error("node lost")` with their
    // synthetic `run_finished` frame persisted in the SAME store transaction
    // (the frame can never be lost to a crash between commit and append);
    // this loop only fans the committed frames out on the DagHub so
    // event-projection UIs see the termination. Fan-out failure degrades to
    // a log line for the same reason as above — the frame stays replayable
    // from the store.
    let swept_runs = match state
        .store
        .converge_lost_dag_runs(now, STALE_AFTER_MS)
        .await
    {
        Ok(runs) => runs,
        Err(e) => return error_500(format!("converge_lost_dag_runs: {e:#}")),
    };
    for c in &swept_runs {
        if let Err(e) = crate::api_nodes_dag::publish_run_finished(
            &state,
            &c.record.id,
            "error",
            Some("node lost"),
            now,
            c.run_finished_seq,
        )
        .await
        {
            tracing::warn!(
                run_id = %c.record.id,
                error = %e,
                "lost-node sweep: failed to emit terminal dag run_finished frame"
            );
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
                        "addr": n.last_addr,
                    })
                })
                .collect();
            Json(json!({ "nodes": nodes })).into_response()
        }
        Err(e) => error_500(format!("list_nodes: {e:#}")),
    }
}

/// POST /api/nodes/register — idempotent by `name`.
///
/// The peer address is recorded for the fleet UI: the request body's declared
/// `addr` wins (NAT/proxy setups), otherwise the TCP source IP.
pub async fn post_register(
    State(state): State<Arc<AppState>>,
    peer: Option<ConnectInfo<std::net::SocketAddr>>,
    Json(body): Json<NodeRegisterRequest>,
) -> Response {
    if body.name.trim().is_empty() {
        return error_400("name must not be empty".into());
    }
    let addr = body
        .addr
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            // Absent only in direct-router tests (oneshot has no connect info).
            peer.as_ref()
                .map(|ci| ci.0.ip().to_string())
                .unwrap_or_else(|| "unknown".to_string())
        });
    let now = chrono::Utc::now().timestamp_millis();
    match state
        .store
        .register_node(
            &body.name,
            body.version.as_deref(),
            body.workdir.as_deref(),
            Some(&addr),
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
    // A BUSY worker never polls claim, so the heartbeat is the only channel
    // guaranteed to reach it: hand out up to HEARTBEAT_CONTROL_BATCH queued
    // control tasks (P3 message relay) alongside the cancel commands.
    let controls = state.controls.pop_many(&id, HEARTBEAT_CONTROL_BATCH).await;
    // DAG cancel piggyback: a busy worker executing a workflow also never
    // polls claim, so its cancellation requests ride this same beat. A store
    // failure must NEVER fail the beat itself (liveness > completeness):
    // degrade to an empty list and let the next beat retry.
    let cancel_run_ids = match state.store.cancelling_dag_runs(&id).await {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!(
                node_id = %id,
                error = %e,
                "cancelling_dag_runs failed; heartbeat continues with empty cancel_run_ids"
            );
            Vec::new()
        }
    };
    match state.store.heartbeat_node(&id, now).await {
        Ok(cancel_task_ids) => Json(NodeHeartbeatResponse {
            server_time_ms: now,
            cancel_task_ids,
            cancel_run_ids,
            controls,
        })
        .into_response(),
        Err(e) => error_500(format!("heartbeat_node: {e:#}")),
    }
}

/// DELETE /api/nodes/:id — remove node + its queue + synthetic sessions.
pub async fn delete_node(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    // Undelivered control tasks die with the node (their browser waiters
    // collapse on their own timeout).
    state.controls.purge_node(&id).await;
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

/// POST /api/nodes/:id/tasks — enqueue one task.
///
/// Default: both ids are fresh ULIDs (`opencoder_session::runner::new_id`) and
/// the synthetic session row is created inside the store's dispatch
/// transaction with `task_type="node"`, which is what hides it from normal
/// session listings.
///
/// With `body.session_id` set (console "continue this dialog") the store binds
/// the task to that EXISTING session instead — no synthetic session row. A
/// missing session answers 400.
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
    let now = chrono::Utc::now().timestamp_millis();
    // Session reuse: a blank/absent session_id means "create synthetic".
    let reused = body
        .session_id
        .clone()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if let Some(session_id) = &reused {
        match state.store.get_session(session_id).await {
            Ok(Some(_)) => {}
            Ok(None) => return error_400(format!("session {session_id} not found")),
            Err(e) => return error_500(format!("get_session: {e:#}")),
        }
    }
    let session_id = reused
        .clone()
        .unwrap_or_else(opencoder_session::runner::new_id);
    let dispatch = if reused.is_some() {
        state.store.dispatch_node_task_for_session(
            &task_id,
            &session_id,
            &id,
            body.title.as_deref(),
            &body.prompt,
            body.agent.as_deref(),
            body.model.as_deref(),
            now,
        )
    } else {
        state.store.dispatch_node_task(
            &task_id,
            &session_id,
            &id,
            body.title.as_deref(),
            &body.prompt,
            body.agent.as_deref(),
            body.model.as_deref(),
            now,
        )
    };
    match dispatch.await {
        Ok(rec) => Json(NodeDispatchResponse {
            task_id: rec.id,
            session_id: rec.session_id,
            status: rec.status.as_str().to_string(),
        })
        .into_response(),
        Err(e) => error_500(format!("dispatch_node_task: {e:#}")),
    }
}
