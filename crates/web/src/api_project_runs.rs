//! `/api/project` run-oriented endpoints: the overview tree, the plan /
//! execute triggers (202 + run_id, background runs owned by the project
//! service), per-todo run history and cancellation. CRUD for the underlying
//! rows lives in `api_project.rs` / `api_project_todos.rs`.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::api_project_util::{error_400, error_500, map_start_err, require_deps};
use crate::AppState;

/// GET /api/project/overview — full goal → milestone → todo tree plus the
/// milestone-less backlog, built by the service.
pub async fn get_overview(State(state): State<Arc<AppState>>) -> Response {
    if let Err(r) = require_deps(&state) {
        return *r;
    }
    match state.project.overview().await {
        Ok(tree) => Json(tree).into_response(),
        Err(e) => error_500(format!("overview: {e:#}")),
    }
}

/// POST /api/project/todos/:id/plan — spawn a plan run for the todo's
/// draft. 202 Accepted + `run_id`: the run completes in the background and
/// lands in `GET /todos/:id/runs`.
pub async fn start_plan(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Option<Json<serde_json::Value>>,
) -> Response {
    if let Err(r) = require_deps(&state) {
        return *r;
    }
    let mut input = body.map(|Json(body)| body).unwrap_or_else(|| json!({}));
    if !input.is_object() {
        return error_400("project input must be an object");
    }
    let run_id = input.as_object_mut().unwrap().remove("run_id");
    if run_id.as_ref().is_some_and(|id| !id.is_string()) {
        return error_400("invalid project run id");
    }
    match state
        .project
        .start_attempt(
            &id,
            opencoder_store::ProjectTodoRunKind::Plan,
            run_id.as_ref().and_then(|id| id.as_str()),
            None,
            input,
        )
        .await
    {
        Ok(run_id) => (StatusCode::ACCEPTED, Json(json!({ "run_id": run_id }))).into_response(),
        Err(e) => map_start_err(e),
    }
}

/// POST /api/project/todos/:id/execute — drive the todo's current plan in a
/// new-or-resumed session. 202 Accepted + `run_id`.
pub async fn start_execute(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Option<Json<serde_json::Value>>,
) -> Response {
    if let Err(r) = require_deps(&state) {
        return *r;
    }
    let mut input = body.map(|Json(body)| body).unwrap_or_else(|| json!({}));
    if !input.is_object() {
        return error_400("project input must be an object");
    }
    let run_id = input.as_object_mut().unwrap().remove("run_id");
    if run_id.as_ref().is_some_and(|id| !id.is_string()) {
        return error_400("invalid project run id");
    }
    match state
        .project
        .start_attempt(
            &id,
            opencoder_store::ProjectTodoRunKind::Execute,
            run_id.as_ref().and_then(|id| id.as_str()),
            None,
            input,
        )
        .await
    {
        Ok(run_id) => (StatusCode::ACCEPTED, Json(json!({ "run_id": run_id }))).into_response(),
        Err(e) => map_start_err(e),
    }
}

/// GET /api/project/todos/:id/runs — the todo's run history, newest
/// version first.
#[derive(serde::Deserialize)]
pub struct RunsQuery {
    before_version: Option<i64>,
}

pub async fn list_todo_runs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<RunsQuery>,
) -> Response {
    if query.before_version.is_some_and(|version| version <= 0) {
        return error_400("invalid project run cursor");
    }
    let deps = match require_deps(&state) {
        Ok(d) => d,
        Err(r) => return *r,
    };
    match deps
        .projects
        .list_todo_runs_page(&id, query.before_version, 20)
        .await
    {
        Ok(page) => Json(page).into_response(),
        Err(e) => error_500(format!("list runs: {e:#}")),
    }
}

/// POST /api/project/runs/:rid/cancel — cancel a live run. Unknown or
/// already-finished ids are NOT errors (idempotent UI action): the response
/// says `{"cancelled": false}`.
pub async fn cancel_run(State(state): State<Arc<AppState>>, Path(rid): Path<String>) -> Response {
    if let Err(r) = require_deps(&state) {
        return *r;
    }
    match state.project.cancel(&rid).await {
        Ok(cancelled) => Json(json!({ "cancelled": cancelled })).into_response(),
        Err(e) => error_500(format!("cancel run: {e:#}")),
    }
}
