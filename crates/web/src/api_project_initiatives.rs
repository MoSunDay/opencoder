use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use opencoder_core::message::now_ms;
use opencoder_store::{ProjectMilestonePatch, ProjectMilestoneRecord, ProjectMilestoneStatus};
use serde::Deserialize;
use serde_json::json;

use crate::api_project_util::{error_400, error_404, error_409, error_500, require_deps};
use crate::AppState;

#[derive(Deserialize)]
pub struct InitiativeQuery {
    pub goal_id: Option<String>,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    Query(query): Query<InitiativeQuery>,
) -> Response {
    let deps = match require_deps(&state) {
        Ok(deps) => deps,
        Err(response) => return *response,
    };
    match deps
        .projects
        .list_initiatives(query.goal_id.as_deref())
        .await
    {
        Ok(items) => Json(json!({ "initiatives": items })).into_response(),
        Err(error) => error_500(error.to_string()),
    }
}

#[derive(Deserialize)]
pub struct CreateBody {
    pub goal_id: Option<String>,
    pub title: String,
    pub detail_md: Option<String>,
    pub sort: Option<i64>,
}

pub async fn create(State(state): State<Arc<AppState>>, Json(body): Json<CreateBody>) -> Response {
    let deps = match require_deps(&state) {
        Ok(deps) => deps,
        Err(response) => return *response,
    };
    let title = body.title.trim();
    if title.is_empty() {
        return error_400("initiative title must not be empty");
    }
    if let Some(goal_id) = &body.goal_id {
        match deps.projects.list_goals().await {
            Ok(goals) if goals.iter().any(|goal| &goal.id == goal_id) => {}
            Ok(_) => return error_404("project not found"),
            Err(error) => return error_500(error.to_string()),
        }
    }
    let now = now_ms();
    let item = ProjectMilestoneRecord {
        id: format!("pi-{}", ulid::Ulid::new()),
        goal_id: body.goal_id,
        title: title.to_owned(),
        detail_md: body.detail_md,
        status: ProjectMilestoneStatus::Planned,
        sort: body.sort.unwrap_or(0),
        created_at: now,
        updated_at: now,
    };
    match deps.projects.create_initiative(&item).await {
        Ok(()) => Json(json!(item)).into_response(),
        Err(error) => error_500(error.to_string()),
    }
}

#[derive(Deserialize)]
pub struct PatchBody {
    #[serde(default, deserialize_with = "crate::api_project_todos::double_option")]
    pub goal_id: Option<Option<String>>,
    pub title: Option<String>,
    pub detail_md: Option<String>,
    pub status: Option<ProjectMilestoneStatus>,
    pub sort: Option<i64>,
}

pub async fn patch(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<PatchBody>,
) -> Response {
    let deps = match require_deps(&state) {
        Ok(deps) => deps,
        Err(response) => return *response,
    };
    let title = body.title.map(|title| title.trim().to_owned());
    if title.as_deref() == Some("") {
        return error_400("initiative title must not be empty");
    }
    if let Some(Some(goal_id)) = &body.goal_id {
        match deps.projects.list_goals().await {
            Ok(goals) if goals.iter().any(|goal| &goal.id == goal_id) => {}
            Ok(_) => return error_404("project not found"),
            Err(error) => return error_500(error.to_string()),
        }
    }
    let change = ProjectMilestonePatch {
        goal_id: body.goal_id,
        title,
        detail_md: body.detail_md,
        status: body.status,
        sort: body.sort,
    };
    match deps.projects.patch_initiative(&id, &change, now_ms()).await {
        Ok(true) => Json(json!({ "ok": true })).into_response(),
        Ok(false) => error_404("initiative not found"),
        Err(error) => error_500(error.to_string()),
    }
}

pub async fn delete(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let deps = match require_deps(&state) {
        Ok(deps) => deps,
        Err(response) => return *response,
    };
    match deps.projects.delete_initiative(&id).await {
        Ok(true) => Json(json!({ "deleted": true })).into_response(),
        Ok(false) => error_404("initiative not found"),
        Err(error) if error.is::<opencoder_store::project::MilestoneNotEmpty>() => {
            error_409("initiative contains TODOs")
        }
        Err(error) => error_500(error.to_string()),
    }
}
