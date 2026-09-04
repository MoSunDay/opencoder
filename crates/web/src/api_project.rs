//! `/api/project` CRUD surface — goals and milestones. Todos live in
//! [`crate::api_project_todos`], run-oriented endpoints (overview / plan /
//! execute / runs / cancel) in [`crate::api_project_runs`]; shared error +
//! deps helpers in [`crate::api_project_util`].
//!
//! IDs are server-generated (`pg-`/`pm-` + ULID). Status enums deserialize
//! via the store records' snake_case serde, so an invalid status string is an
//! automatic 4xx axum Json rejection.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use opencoder_core::message::now_ms;
use opencoder_store::{
    ProjectGoalPatch, ProjectGoalRecord, ProjectGoalStatus, ProjectMilestonePatch,
    ProjectMilestoneRecord, ProjectMilestoneStatus,
};

use crate::api_project_util::{error_400, error_404, error_500, rec_list, require_deps, to_json};
use crate::AppState;

// ── goals ──────────────────────────────────────────────────────────────

/// GET /api/project/goals — all goals, `sort` then `created_at` order.
pub async fn list_goals(State(state): State<Arc<AppState>>) -> Response {
    let deps = match require_deps(&state) {
        Ok(d) => d,
        Err(r) => return *r,
    };
    match deps.projects.list_goals().await {
        Ok(goals) => Json(json!({ "goals": rec_list(goals) })).into_response(),
        Err(e) => error_500(format!("list goals: {e:#}")),
    }
}

#[derive(Deserialize)]
pub struct CreateGoalBody {
    pub title: String,
    #[serde(default)]
    pub detail_md: Option<String>,
    #[serde(default)]
    pub sort: Option<i64>,
}

/// POST /api/project/goals — new goal in `active` state. Plain 200 + the
/// created record (matches api_envs' create convention).
pub async fn create_goal(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateGoalBody>,
) -> Response {
    let deps = match require_deps(&state) {
        Ok(d) => d,
        Err(r) => return *r,
    };
    let title = match trimmed(body.title) {
        Ok(t) => t,
        Err(()) => return error_400("goal title must not be empty"),
    };
    let now = now_ms();
    let rec = ProjectGoalRecord {
        id: format!("pg-{}", ulid::Ulid::new()),
        title,
        detail_md: body.detail_md,
        status: ProjectGoalStatus::Active,
        sort: body.sort.unwrap_or(0),
        created_at: now,
        updated_at: now,
    };
    match deps.projects.create_goal(&rec).await {
        Ok(()) => Json(to_json(&rec)).into_response(),
        Err(e) => error_500(format!("create goal: {e:#}")),
    }
}

#[derive(Deserialize)]
pub struct PatchGoalBody {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub detail_md: Option<String>,
    #[serde(default)]
    pub status: Option<ProjectGoalStatus>,
    #[serde(default)]
    pub sort: Option<i64>,
}

/// PATCH /api/project/goals/:id — partial update; unknown id → 404, a
/// supplied-but-blank title → 400, an absent field stays unchanged.
pub async fn patch_goal(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<PatchGoalBody>,
) -> Response {
    let deps = match require_deps(&state) {
        Ok(d) => d,
        Err(r) => return *r,
    };
    let title = match body.title.map(trimmed) {
        None => None,
        Some(Ok(t)) => Some(t),
        Some(Err(())) => return error_400("goal title must not be empty"),
    };
    let patch = ProjectGoalPatch {
        title,
        detail_md: body.detail_md,
        status: body.status,
        sort: body.sort,
    };
    match deps.projects.patch_goal(&id, &patch, now_ms()).await {
        Ok(true) => Json(json!({ "ok": true })).into_response(),
        Ok(false) => error_404(format!("goal not found: {id}")),
        Err(e) => error_500(format!("patch goal: {e:#}")),
    }
}

/// DELETE /api/project/goals/:id — cascades milestones → todos → runs.
pub async fn delete_goal(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let deps = match require_deps(&state) {
        Ok(d) => d,
        Err(r) => return *r,
    };
    match deps.projects.delete_goal(&id).await {
        Ok(true) => Json(json!({ "deleted": true })).into_response(),
        Ok(false) => error_404(format!("goal not found: {id}")),
        Err(e) => error_500(format!("delete goal: {e:#}")),
    }
}

// ── milestones ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct MilestoneQuery {
    pub goal_id: Option<String>,
}

/// GET /api/project/milestones?goal_id= — one goal's milestones, or across
/// all goals when the parameter is absent.
pub async fn list_milestones(
    State(state): State<Arc<AppState>>,
    Query(q): Query<MilestoneQuery>,
) -> Response {
    let deps = match require_deps(&state) {
        Ok(d) => d,
        Err(r) => return *r,
    };
    match deps.projects.list_milestones(q.goal_id.as_deref()).await {
        Ok(items) => Json(json!({ "milestones": rec_list(items) })).into_response(),
        Err(e) => error_500(format!("list milestones: {e:#}")),
    }
}

#[derive(Deserialize)]
pub struct CreateMilestoneBody {
    pub goal_id: String,
    pub title: String,
    #[serde(default)]
    pub detail_md: Option<String>,
    #[serde(default)]
    pub sort: Option<i64>,
}

/// POST /api/project/milestones — new `planned` milestone under an existing
/// goal; unknown goal_id → 404 (same contract as patch re-parenting).
pub async fn create_milestone(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateMilestoneBody>,
) -> Response {
    let deps = match require_deps(&state) {
        Ok(d) => d,
        Err(r) => return *r,
    };
    let title = match trimmed(body.title) {
        Ok(t) => t,
        Err(()) => return error_400("milestone title must not be empty"),
    };
    match deps.projects.list_goals().await {
        Ok(goals) if goals.iter().any(|g| g.id == body.goal_id) => {}
        Ok(_) => return error_404(format!("goal not found: {}", body.goal_id)),
        Err(e) => return error_500(format!("verify goal: {e:#}")),
    }
    let now = now_ms();
    let rec = ProjectMilestoneRecord {
        id: format!("pm-{}", ulid::Ulid::new()),
        goal_id: body.goal_id,
        title,
        detail_md: body.detail_md,
        status: ProjectMilestoneStatus::Planned,
        sort: body.sort.unwrap_or(0),
        created_at: now,
        updated_at: now,
    };
    match deps.projects.create_milestone(&rec).await {
        Ok(()) => Json(to_json(&rec)).into_response(),
        Err(e) => error_500(format!("create milestone: {e:#}")),
    }
}

#[derive(Deserialize)]
pub struct PatchMilestoneBody {
    #[serde(default)]
    pub goal_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub detail_md: Option<String>,
    #[serde(default)]
    pub status: Option<ProjectMilestoneStatus>,
    #[serde(default)]
    pub sort: Option<i64>,
}

/// PATCH /api/project/milestones/:id — partial update; re-parenting to an
/// unknown goal → 404.
pub async fn patch_milestone(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<PatchMilestoneBody>,
) -> Response {
    let deps = match require_deps(&state) {
        Ok(d) => d,
        Err(r) => return *r,
    };
    let title = match body.title.map(trimmed) {
        None => None,
        Some(Ok(t)) => Some(t),
        Some(Err(())) => return error_400("milestone title must not be empty"),
    };
    if let Some(goal_id) = &body.goal_id {
        match deps.projects.list_goals().await {
            Ok(goals) if goals.iter().any(|g| &g.id == goal_id) => {}
            Ok(_) => return error_404(format!("goal not found: {goal_id}")),
            Err(e) => return error_500(format!("verify goal: {e:#}")),
        }
    }
    let patch = ProjectMilestonePatch {
        goal_id: body.goal_id,
        title,
        detail_md: body.detail_md,
        status: body.status,
        sort: body.sort,
    };
    match deps.projects.patch_milestone(&id, &patch, now_ms()).await {
        Ok(true) => Json(json!({ "ok": true })).into_response(),
        Ok(false) => error_404(format!("milestone not found: {id}")),
        Err(e) => error_500(format!("patch milestone: {e:#}")),
    }
}

/// DELETE /api/project/milestones/:id — cascades the milestone's todos and
/// their runs (todos are deleted, not re-parented to the backlog).
pub async fn delete_milestone(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let deps = match require_deps(&state) {
        Ok(d) => d,
        Err(r) => return *r,
    };
    match deps.projects.delete_milestone(&id).await {
        Ok(true) => Json(json!({ "deleted": true })).into_response(),
        Ok(false) => error_404(format!("milestone not found: {id}")),
        Err(e) => error_500(format!("delete milestone: {e:#}")),
    }
}

/// Trim a required-title candidate; blank-after-trim is `Err(())`.
fn trimmed(s: String) -> Result<String, ()> {
    let t = s.trim().to_string();
    if t.is_empty() {
        Err(())
    } else {
        Ok(t)
    }
}
