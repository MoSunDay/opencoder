//! Node-task operation HTTP surface (`/api/nodes/tasks*` + cancel route).
//!
//! Claim / live-event upload / terminal status / cancellation, plus the read
//! half (single task / fleet-wide list / session reverse lookup). Pure handlers:
//! every transition delegates to the [`Store`] node API, every closure event is
//! persisted FIRST and then fanned out to the [`NodeHub`] so a browser SSE
//! stream replays identical bytes whether it was attached or not.
//!
//! [`NodeHub`]: crate::nodes_state::NodeHub

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use opencoder_core::node_protocol::{ClaimResponse, ClaimedTask, NodeEventBatch, NodeStatusReport};
use opencoder_core::SseEvt;
use opencoder_store::{EventKind, NodeTaskStatus, SessionEventRecord};
use serde::Deserialize;
use serde_json::json;

use crate::api::{error_400, error_404, error_409, error_500};
use crate::AppState;

#[derive(Deserialize)]
pub struct ClaimQuery {
    pub node_id: String,
}

/// GET /api/nodes/tasks/claim?node_id= — FIFO single-active dispatch.
///
/// Reply is the [`ClaimResponse`] envelope: a durable task is preferred; when
/// the store has nothing due, a queued control task ([`ControlTask`], P3
/// message relay) rides along instead. `200` carries `task`, `control` or
/// both; `204` means neither was due (idle/running, nothing queued).
pub async fn claim(State(state): State<Arc<AppState>>, Query(q): Query<ClaimQuery>) -> Response {
    let now = chrono::Utc::now().timestamp_millis();
    let claimed = match state.store.claim_next_node_task(&q.node_id, now).await {
        Ok(Some(t)) => Some(ClaimedTask {
            task_id: t.id,
            session_id: t.session_id,
            title: t.title,
            prompt: t.prompt,
            agent: t.agent,
            model: t.model,
            created_at: t.created_at,
        }),
        Ok(None) => None,
        Err(e) => return error_500(format!("claim_next_node_task: {e:#}")),
    };
    if claimed.is_some() {
        // Durable work wins the single-active slot; control tasks wait for
        // the busy worker's heartbeat instead of stacking up here.
        return Json(ClaimResponse {
            task: claimed,
            control: None,
        })
        .into_response();
    }
    let control = state.controls.pop(&q.node_id).await;
    if control.is_none() {
        return StatusCode::NO_CONTENT.into_response();
    }
    Json(ClaimResponse {
        task: None,
        control,
    })
    .into_response()
}

/// SSE-kind string → coarse DB kind. Unknown granular names degrade to
/// `Step` so a version-skewed worker never breaks durability of its upload.
fn parse_event_kind(sse_kind: &str) -> EventKind {
    match sse_kind {
        "text_delta" => EventKind::TextDelta,
        "tool_start" => EventKind::ToolStart,
        "tool_end" => EventKind::ToolEnd,
        "done" => EventKind::Done,
        "error" => EventKind::Error,
        "interrupted" => EventKind::Interrupted,
        _ => EventKind::Step,
    }
}

/// POST /api/nodes/tasks/:tid/events — persist a worker's event batch onto the
/// task's synthetic session, then re-broadcast each row to live subscribers
/// with its assigned seq.
///
/// Guard order: unknown task ⇒ 404, non-`running/cancelling` ⇒ 409 (no queue
/// noise should ever be appended outside an execution window).
pub async fn post_events(
    State(state): State<Arc<AppState>>,
    Path(tid): Path<String>,
    Json(batch): Json<NodeEventBatch>,
) -> Response {
    let task = match state.store.get_node_task(&tid).await {
        Ok(Some(t)) => t,
        Ok(None) => return error_404("task not found"),
        Err(e) => return error_500(format!("get_node_task: {e:#}")),
    };
    if !matches!(
        task.status,
        NodeTaskStatus::Running | NodeTaskStatus::Cancelling
    ) {
        return error_409("task is not running; refusing event upload");
    }
    if batch.events.is_empty() {
        return Json(json!({ "appended": 0 })).into_response();
    }
    let records: Vec<SessionEventRecord> = batch
        .events
        .iter()
        .map(|e| SessionEventRecord {
            session_id: task.session_id.clone(),
            kind: parse_event_kind(&e.sse_kind),
            payload: e.payload.clone(),
            ts: e.ts,
            seq: None,
            sse_kind: Some(e.sse_kind.clone()),
        })
        .collect();
    let count = records.len();
    let seqs = match state.store.append_events(&records).await {
        Ok(s) => s,
        Err(e) => return error_500(format!("append_events: {e:#}")),
    };
    for (ev, seq) in records.iter().zip(seqs) {
        state
            .nodes
            .broadcast(
                &task.session_id,
                SseEvt {
                    kind: ev.sse_kind.clone().unwrap_or_default(),
                    data: ev.payload.clone(),
                    ts: ev.ts,
                    seq: Some(seq),
                },
            )
            .await;
    }
    Json(json!({ "appended": count })).into_response()
}

/// One canonical terminal event ("closure"): persisted first, then fanned out
/// on the hub so every SSE stream ends in `done`/`error` no matter how the run
/// stopped. Bundled in a struct to keep the emit call single-subject.
pub(crate) struct ClosureEvent<'a> {
    pub session_id: &'a str,
    pub kind: EventKind,
    pub sse_kind: &'static str,
    pub task_id: &'a str,
    pub ok: bool,
    pub error: Option<&'a str>,
    /// Collapsed cancellation (interrupt semantics): sets `payload.cancel`.
    pub cancel: bool,
}

/// Persist one closure event for its synthetic session and fan it out to live
/// subscribers. Returns the persisted seq so callers can reconcile wire ↔ store.
pub(crate) async fn emit_closure(
    state: &AppState,
    ev: ClosureEvent<'_>,
    ts: i64,
) -> Result<i64, String> {
    let mut payload = json!({ "ok": ev.ok, "error": ev.error, "task_id": ev.task_id });
    if ev.cancel {
        payload["cancel"] = json!(true);
    }
    let rec = SessionEventRecord {
        session_id: ev.session_id.to_string(),
        kind: ev.kind,
        payload: payload.clone(),
        ts,
        seq: None,
        sse_kind: Some(ev.sse_kind.to_string()),
    };
    let seqs = state
        .store
        .append_events(std::slice::from_ref(&rec))
        .await
        .map_err(|e| format!("append_events: {e:#}"))?;
    let seq = seqs.first().copied().unwrap_or(0);
    state
        .nodes
        .broadcast(
            ev.session_id,
            SseEvt {
                kind: ev.sse_kind.to_string(),
                data: payload,
                ts,
                seq: Some(seq),
            },
        )
        .await;
    Ok(seq)
}

/// POST /api/nodes/tasks/:tid/status — worker reports a terminal state.
///
/// Persists the transition, then appends + broadcasts exactly one closure
/// event so every stream ends in the canonical `done`/`error` SSE kind (the
/// front-end protocol has no `cancelled` event name; a cancellation collapses
/// to `done` with `payload.cancel = true`, mirroring interrupt semantics).
pub async fn post_status(
    State(state): State<Arc<AppState>>,
    Path(tid): Path<String>,
    Json(report): Json<NodeStatusReport>,
) -> Response {
    if !report.validate() {
        return error_400(format!(
            "invalid status {:?}: expected \"done\" | \"error\" | \"cancelled\"",
            report.status
        ));
    }
    // Existence pre-check keeps unknown ids a 404 instead of relying on the
    // store's string-y bail message.
    let task = match state.store.get_node_task(&tid).await {
        Ok(Some(t)) => t,
        Ok(None) => return error_404("task not found"),
        Err(e) => return error_500(format!("get_node_task: {e:#}")),
    };
    let st = match report.status.as_str() {
        "done" => NodeTaskStatus::Done,
        "cancelled" => NodeTaskStatus::Cancelled,
        _ => NodeTaskStatus::Error,
    };
    let now = chrono::Utc::now().timestamp_millis();
    if let Err(e) = state
        .store
        .update_node_task_status(&tid, st, report.error.as_deref(), now)
        .await
    {
        // Illegal transitions (e.g. double-terminal) surface as conflicts.
        return error_409(&format!("update_node_task_status: {e:#}"));
    }
    let ok = st == NodeTaskStatus::Done;
    let closure = ClosureEvent {
        session_id: &task.session_id,
        kind: if st == NodeTaskStatus::Error {
            EventKind::Error
        } else {
            EventKind::Done
        },
        sse_kind: if st == NodeTaskStatus::Error {
            "error"
        } else {
            "done"
        },
        task_id: &tid,
        ok,
        error: report.error.as_deref(),
        cancel: st == NodeTaskStatus::Cancelled,
    };
    if let Err(e) = emit_closure(&state, closure, now).await {
        return error_500(e);
    }
    Json(json!({ "ok": true, "task_id": tid, "status": report.status })).into_response()
}

/// POST /api/nodes/:node_id/tasks/:tid/cancel — request task abortion.
///
/// - `pending`: collapses immediately (`cancelled` + closure event) since it
///   never started — queue removal is the whole job.
/// - `running | cancelling`: `202 cancelling`; the actual stop travels to the
///   worker via its next heartbeat's `cancel_task_ids`.
/// - terminal / already-cancelling-stale / unknown: `409` or `404`.
pub async fn cancel_task(
    State(state): State<Arc<AppState>>,
    Path((node_id, tid)): Path<(String, String)>,
) -> Response {
    let task = match state.store.get_node_task(&tid).await {
        Ok(Some(t)) => t,
        Ok(None) => return error_404("task not found"),
        Err(e) => return error_500(format!("get_node_task: {e:#}")),
    };
    if task.node_id != node_id {
        return error_404("task not found on this node");
    }
    match state.store.request_node_task_cancel(&tid).await {
        Ok(Some(NodeTaskStatus::Pending)) => {
            let now = chrono::Utc::now().timestamp_millis();
            if let Err(e) = state
                .store
                .update_node_task_status(&tid, NodeTaskStatus::Cancelled, None, now)
                .await
            {
                return error_409(&format!("update_node_task_status: {e:#}"));
            }
            let closure = ClosureEvent {
                session_id: &task.session_id,
                kind: EventKind::Done,
                sse_kind: "done",
                task_id: &tid,
                ok: true,
                error: None,
                cancel: true,
            };
            if let Err(e) = emit_closure(&state, closure, now).await {
                return error_500(e);
            }
            Json(json!({ "ok": true, "phase": "cancelled" })).into_response()
        }
        Ok(Some(_)) => (
            StatusCode::ACCEPTED,
            Json(json!({ "ok": true, "phase": "cancelling" })),
        )
            .into_response(),
        Ok(None) => error_409("not cancellable"),
        Err(e) => error_500(format!("request_node_task_cancel: {e:#}")),
    }
}

// ── read half: task status pulls (parity with the TUI fleet view) ─────────

/// Wire status token → enum. `NodeTaskStatus::parse` silently falls back to
/// `Pending`, which is wrong for a user-supplied query filter — unknown values
/// must surface as 400 instead of silently matching the pending queue.
fn parse_status_filter(raw: &str) -> Option<NodeTaskStatus> {
    match raw {
        "pending" => Some(NodeTaskStatus::Pending),
        "running" => Some(NodeTaskStatus::Running),
        "done" => Some(NodeTaskStatus::Done),
        "error" => Some(NodeTaskStatus::Error),
        "cancelled" => Some(NodeTaskStatus::Cancelled),
        "cancelling" => Some(NodeTaskStatus::Cancelling),
        _ => None,
    }
}

#[derive(Deserialize)]
pub struct GlobalTasksQuery {
    pub node_id: Option<String>,
    pub status: Option<String>,
    pub limit: Option<u32>,
}

/// GET /api/nodes/tasks?status=&node_id=&limit= — fleet-wide task list.
///
/// FIFO order (oldest first, identical to the claim drain order). The static
/// sibling route `/api/nodes/tasks/claim` keeps priority in matchit, so worker
/// claim polling is unaffected. Default `limit` 200.
pub async fn list_all_tasks(
    State(state): State<Arc<AppState>>,
    Query(q): Query<GlobalTasksQuery>,
) -> Response {
    let status = match q.status.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => match parse_status_filter(raw) {
            Some(st) => Some(st),
            None => return error_400(format!("unknown status filter '{raw}'")),
        },
        None => None,
    };
    let node_id = q
        .node_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let limit = q.limit.unwrap_or(200).clamp(1, 1000);
    match state
        .store
        .list_node_tasks_filtered(node_id, status, limit)
        .await
    {
        Ok(tasks) => Json(json!({ "tasks": tasks })).into_response(),
        Err(e) => error_500(format!("list_node_tasks_filtered: {e:#}")),
    }
}

/// GET /api/nodes/tasks/:tid — one task record plus `last_event_seq`, the
/// SSE bootstrap convenience (pass it as `?after=` on the events stream to
/// skip the already-persisted prefix).
pub async fn get_task(State(state): State<Arc<AppState>>, Path(tid): Path<String>) -> Response {
    let task = match state.store.get_node_task(&tid).await {
        Ok(Some(t)) => t,
        Ok(None) => return error_404("task not found"),
        Err(e) => return error_500(format!("get_node_task: {e:#}")),
    };
    let last_event_seq = state
        .store
        .last_event_seq(&task.session_id)
        .await
        .unwrap_or(0);
    let mut body = match serde_json::to_value(&task) {
        Ok(v) => v,
        Err(e) => return error_500(format!("serialize task: {e}")),
    };
    body["last_event_seq"] = json!(last_event_seq);
    Json(body).into_response()
}

/// GET /api/sessions/:id/task — reverse lookup from the synthetic session to
/// its node task. 404 for ordinary sessions (they legitimately have none).
pub async fn get_session_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    match state.store.get_node_task_by_session(&id).await {
        Ok(Some(t)) => Json(t).into_response(),
        Ok(None) => error_404("no node task for this session"),
        Err(e) => error_500(format!("get_node_task_by_session: {e:#}")),
    }
}
