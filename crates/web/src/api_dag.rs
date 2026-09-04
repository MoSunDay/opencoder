//! Browser-facing DAG HTTP surface (`/api/dag/*`): definition CRUD, run
//! dispatch and run reads/cancel. Pure handlers over the [`Store`] DAG API —
//! the server stores and forwards only, it never executes a workflow.
//!
//! The agent channel (claim / event upload / status) lives in
//! `api_nodes_dag.rs`; the browser event stream lives in `sse_dag.rs`.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use opencoder_dag::{
    validate, DagDefUpsertRequest, DagDefView, DagDispatchRequest, DagDispatchResponse,
    DagRunStatus, DagRunView, DagSpec,
};
use opencoder_store::{DagDefRecord, DagRunRecord};
use serde::Deserialize;
use serde_json::json;

use crate::api::{error_400, error_404, error_409, error_500};
use crate::AppState;

/// Store row → wire view (parses the lazily-stored `spec_json`).
fn def_view(rec: &DagDefRecord) -> Result<DagDefView, String> {
    let spec: DagSpec =
        serde_json::from_str(&rec.spec_json).map_err(|e| format!("parse dag spec: {e}"))?;
    Ok(DagDefView {
        id: rec.id.clone(),
        name: rec.name.clone(),
        spec,
        created_at: rec.created_at,
        updated_at: rec.updated_at,
    })
}

fn run_view(rec: &DagRunRecord) -> DagRunView {
    DagRunView {
        id: rec.id.clone(),
        dag_id: rec.dag_id.clone(),
        name: rec.name.clone(),
        node_id: rec.node_id.clone(),
        status: rec.status.as_str().to_string(),
        error: rec.error.clone(),
        created_at: rec.created_at,
        claimed_at: rec.claimed_at,
        finished_at: rec.finished_at,
    }
}

/// POST /api/dag/defs — upsert by `spec.name` (id/created_at stay stable
/// across edits). Invalid specs are rejected with the aggregated problem
/// list from `opencoder_dag::validate`.
pub async fn post_def(
    State(state): State<Arc<AppState>>,
    Json(body): Json<DagDefUpsertRequest>,
) -> Response {
    if let Err(problems) = validate(&body.spec) {
        return error_400(problems.join("; "));
    }
    let now = chrono::Utc::now().timestamp_millis();
    let spec_json = match serde_json::to_string(&body.spec) {
        Ok(s) => s,
        Err(e) => return error_500(format!("serialize dag spec: {e}")),
    };
    let def = DagDefRecord {
        id: ulid::Ulid::new().to_string(),
        name: body.spec.name.clone(),
        spec_json,
        created_at: now,
        updated_at: now,
    };
    if let Err(e) = state.store.upsert_dag_def(&def).await {
        return error_500(format!("upsert_dag_def: {e:#}"));
    }
    // The name is the conflict key: on a re-publish the row keeps its
    // ORIGINAL id/created_at, so the reply is read back rather than echoed.
    let stored = match state.store.list_dag_defs().await {
        Ok(defs) => defs.into_iter().find(|d| d.name == def.name),
        Err(e) => return error_500(format!("list_dag_defs: {e:#}")),
    };
    let Some(stored) = stored else {
        return error_500("upsert_dag_def: row vanished after upsert".into());
    };
    match def_view(&stored) {
        Ok(v) => Json(v).into_response(),
        Err(e) => error_500(e),
    }
}

/// GET /api/dag/defs — all definitions ordered by name.
pub async fn list_defs(State(state): State<Arc<AppState>>) -> Response {
    match state.store.list_dag_defs().await {
        Ok(defs) => {
            let mut views = Vec::with_capacity(defs.len());
            for d in &defs {
                match def_view(d) {
                    Ok(v) => views.push(v),
                    Err(e) => return error_500(format!("def {}: {e}", d.id)),
                }
            }
            Json(views).into_response()
        }
        Err(e) => error_500(format!("list_dag_defs: {e:#}")),
    }
}

/// GET /api/dag/defs/:id — one definition.
pub async fn get_def(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.store.get_dag_def(&id).await {
        Ok(Some(d)) => match def_view(&d) {
            Ok(v) => Json(v).into_response(),
            Err(e) => error_500(e),
        },
        Ok(None) => error_404("dag def not found"),
        Err(e) => error_500(format!("get_dag_def: {e:#}")),
    }
}

/// DELETE /api/dag/defs/:id — drop a definition (in-flight runs keep their
/// spec snapshot, so deleting never disturbs them).
pub async fn delete_def(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.store.get_dag_def(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return error_404("dag def not found"),
        Err(e) => return error_500(format!("get_dag_def: {e:#}")),
    }
    match state.store.delete_dag_def(&id).await {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => error_500(format!("delete_dag_def: {e:#}")),
    }
}

/// POST /api/dag/defs/:id/dispatch — enqueue a run. The def must exist
/// (404); a given `node_id` pins the run to that node and must exist (400,
/// same wording as the node-task dispatch path).
pub async fn dispatch(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<DagDispatchRequest>,
) -> Response {
    let def = match state.store.get_dag_def(&id).await {
        Ok(Some(d)) => d,
        Ok(None) => return error_404("dag def not found"),
        Err(e) => return error_500(format!("get_dag_def: {e:#}")),
    };
    // Blank node_id means "any node" — same blank-filter as session reuse in
    // the node-task dispatch.
    let pinned = body
        .node_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if let Some(node_id) = &pinned {
        match state.store.get_node(node_id).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                return error_400(format!("node {node_id} does not exist"));
            }
            Err(e) => return error_500(format!("get_node: {e:#}")),
        }
    }
    let now = chrono::Utc::now().timestamp_millis();
    let run = DagRunRecord {
        id: ulid::Ulid::new().to_string(),
        dag_id: def.id.clone(),
        name: def.name.clone(),
        spec_json: def.spec_json.clone(),
        node_id: pinned,
        status: DagRunStatus::Pending,
        error: None,
        created_at: now,
        claimed_at: None,
        finished_at: None,
    };
    match state.store.dispatch_dag_run(&run).await {
        Ok(rec) => Json(DagDispatchResponse { run_id: rec.id }).into_response(),
        Err(e) => error_500(format!("dispatch_dag_run: {e:#}")),
    }
}

#[derive(Deserialize, Default)]
pub struct RunsQuery {
    pub limit: Option<u32>,
}

/// GET /api/dag/runs?limit=N — newest first, default 50, capped at 200.
pub async fn list_runs(State(state): State<Arc<AppState>>, Query(q): Query<RunsQuery>) -> Response {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    match state.store.list_dag_runs(limit).await {
        Ok(runs) => Json(runs.iter().map(run_view).collect::<Vec<_>>()).into_response(),
        Err(e) => error_500(format!("list_dag_runs: {e:#}")),
    }
}

/// GET /api/dag/runs/:id — one run row.
pub async fn get_run(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.store.get_dag_run(&id).await {
        Ok(Some(r)) => Json(run_view(&r)).into_response(),
        Ok(None) => error_404("dag run not found"),
        Err(e) => error_500(format!("get_dag_run: {e:#}")),
    }
}

/// POST /api/dag/runs/:id/cancel — request abortion. `pending` collapses
/// straight to `cancelled` (nothing claimed it); a live run flips to
/// `cancelling`, which the node observes on its next heartbeat. Terminal
/// runs refuse with 409 (state frozen).
pub async fn cancel_run(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let run = match state.store.get_dag_run(&id).await {
        Ok(Some(r)) => r,
        Ok(None) => return error_404("dag run not found"),
        Err(e) => return error_500(format!("get_dag_run: {e:#}")),
    };
    let now = chrono::Utc::now().timestamp_millis();
    match state.store.cancel_dag_run(&id, now).await {
        Ok(()) => {
            let phase = if run.status == DagRunStatus::Pending {
                "cancelled"
            } else {
                "cancelling"
            };
            Json(json!({ "ok": true, "phase": phase })).into_response()
        }
        Err(e) => {
            let msg = format!("cancel_dag_run: {e:#}");
            if msg.contains("not found") {
                error_404("dag run not found")
            } else if msg.contains("already terminal") {
                // The store's illegal-move text verbatim → 409.
                error_409(&msg)
            } else {
                error_500(msg)
            }
        }
    }
}
