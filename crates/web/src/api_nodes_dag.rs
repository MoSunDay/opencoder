//! Agent-channel DAG endpoints (`/api/nodes/dag/*`): FIFO claim, live-event
//! upload and terminal status — the same store-and-forward trio as
//! `api_nodes_ops.rs`, minus the synthetic-session layer (a DAG run carries
//! its own `spec_json` snapshot). Every uploaded frame is persisted FIRST,
//! then published to the run's [`DagHub`] so SSE streams replay identical
//! bytes whether they were attached or not.
//!
//! [`DagHub`]: crate::dag_state::DagHub

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use opencoder_dag::protocol::DAG_EVENT_KINDS;
use opencoder_dag::{DagClaimedRun, DagEventBatch, DagEventView, DagRunStatus, DagStatusReport};
use opencoder_store::DagEventRecord;
use serde::Deserialize;
use serde_json::json;

use crate::api::{error_400, error_404, error_409, error_500};
use crate::dag_state::shared_dag_hub;
use crate::AppState;

#[derive(Deserialize)]
pub struct DagClaimQuery {
    pub node_id: String,
}

/// GET /api/nodes/dag/claim?node_id= — FIFO single-active-run dispatch.
/// `200` carries the run plus its spec snapshot; `204` means nothing is due.
pub async fn claim(State(state): State<Arc<AppState>>, Query(q): Query<DagClaimQuery>) -> Response {
    let now = chrono::Utc::now().timestamp_millis();
    match state.store.claim_next_dag_run(&q.node_id, now).await {
        Ok(Some(run)) => {
            let spec = match serde_json::from_str(&run.spec_json) {
                Ok(s) => s,
                Err(e) => return error_500(format!("parse dag spec snapshot for {}: {e}", run.id)),
            };
            Json(DagClaimedRun {
                run_id: run.id,
                dag_id: run.dag_id,
                spec,
                created_at: run.created_at,
            })
            .into_response()
        }
        Ok(None) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error_500(format!("claim_next_dag_run: {e:#}")),
    }
}

/// POST /api/nodes/dag/runs/:rid/events — persist a node's event batch, then
/// publish each row (with its assigned seq) to the run's hub subscribers.
///
/// Guard order: unknown run ⇒ 404; body `run_id` mismatch ⇒ 400; any unknown
/// event kind ⇒ 400 (the whole batch is rejected — partial persist of an
/// invalid batch would corrupt the step projection).
pub async fn post_events(
    State(state): State<Arc<AppState>>,
    Path(rid): Path<String>,
    Json(batch): Json<DagEventBatch>,
) -> Response {
    match state.store.get_dag_run(&rid).await {
        Ok(Some(_)) => {}
        Ok(None) => return error_404("dag run not found"),
        Err(e) => return error_500(format!("get_dag_run: {e:#}")),
    }
    if batch.run_id != rid {
        return error_400(format!(
            "batch run_id {} does not match path run_id {rid}",
            batch.run_id
        ));
    }
    if let Some(bad) = batch
        .events
        .iter()
        .find(|e| !DAG_EVENT_KINDS.contains(&e.kind.as_str()))
    {
        return error_400(format!(
            "unknown dag event kind {:?}: expected one of {:?}",
            bad.kind, DAG_EVENT_KINDS
        ));
    }
    if batch.events.is_empty() {
        return Json(json!({ "accepted": 0 })).into_response();
    }
    let records: Vec<DagEventRecord> = batch
        .events
        .iter()
        .map(|e| DagEventRecord {
            seq: None,
            run_id: rid.clone(),
            kind: e.kind.clone(),
            step: e.step.clone(),
            payload: e.payload.clone(),
            at_ms: e.at_ms,
        })
        .collect();
    let seqs = match state.store.append_dag_events(&records).await {
        Ok(s) => s,
        Err(e) => return error_500(format!("append_dag_events: {e:#}")),
    };
    let hub = shared_dag_hub();
    for (ev, seq) in records.iter().zip(seqs) {
        hub.publish(
            &rid,
            DagEventView {
                seq,
                kind: ev.kind.clone(),
                step: ev.step.clone(),
                payload: ev.payload.clone(),
                at_ms: ev.at_ms,
            },
        )
        .await;
    }
    Json(json!({ "accepted": batch.events.len() })).into_response()
}

/// POST /api/nodes/dag/runs/:rid/status — terminal report. Persists the
/// transition, then appends + publishes one synthetic `run_finished` event so
/// event-projection UIs see completion without polling the run row.
pub async fn post_status(
    State(state): State<Arc<AppState>>,
    Path(rid): Path<String>,
    Json(report): Json<DagStatusReport>,
) -> Response {
    let status = match report.status.as_str() {
        "done" => DagRunStatus::Done,
        "error" => DagRunStatus::Error,
        "cancelled" => DagRunStatus::Cancelled,
        other => {
            return error_400(format!(
                "invalid status {other:?}: expected \"done\" | \"error\" | \"cancelled\""
            ))
        }
    };
    match state.store.get_dag_run(&rid).await {
        Ok(Some(_)) => {}
        Ok(None) => return error_404("dag run not found"),
        Err(e) => return error_500(format!("get_dag_run: {e:#}")),
    }
    let now = chrono::Utc::now().timestamp_millis();
    if let Err(e) = state
        .store
        .update_dag_run_status(&rid, status, report.error.as_deref(), now)
        .await
    {
        let msg = format!("update_dag_run_status: {e:#}");
        if msg.contains("not found") {
            return error_404("dag run not found");
        }
        // Illegal transitions (e.g. double-terminal) surface as conflicts;
        // anything else is a store failure.
        if !msg.contains("illegal") {
            return error_500(msg);
        }
        return error_409(&msg);
    }
    if let Err(e) = emit_run_finished(
        &state,
        &rid,
        report.status.as_str(),
        report.error.as_deref(),
        now,
    )
    .await
    {
        return error_500(e);
    }
    Json(json!({ "ok": true, "run_id": rid, "status": report.status })).into_response()
}

/// Persist + fan out the synthetic `run_finished` frame for a terminal
/// transition (status report or lost-node sweep). Durable first, live second;
/// the assigned `seq` rides on the wire frame so SSE reconnects resume
/// cleanly. Shared by `api_nodes::list_nodes` (sweep) and [`post_status`].
pub(crate) async fn emit_run_finished(
    state: &AppState,
    run_id: &str,
    status: &str,
    error: Option<&str>,
    at_ms: i64,
) -> Result<(), String> {
    let mut payload = json!({ "status": status });
    if let Some(err) = error {
        payload["error"] = json!(err);
    }
    let record = DagEventRecord {
        seq: None,
        run_id: run_id.to_string(),
        kind: "run_finished".to_string(),
        step: None,
        payload: payload.clone(),
        at_ms,
    };
    let seqs = state
        .store
        .append_dag_events(std::slice::from_ref(&record))
        .await
        .map_err(|e| format!("append_dag_events: {e:#}"))?;
    let seq = seqs.first().copied().unwrap_or(0);
    shared_dag_hub()
        .publish(
            run_id,
            DagEventView {
                seq,
                kind: "run_finished".to_string(),
                step: None,
                payload,
                at_ms,
            },
        )
        .await;
    Ok(())
}
