use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use opencoder_core::message::now_ms;
use opencoder_store::{ProjectInitiativePatch, ProjectInitiativeRecord};
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
        Err(error) => crate::api_project_people::write_error(error),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateBody {
    #[serde(flatten)]
    pub people: opencoder_store::project::people::ProjectPeople,
    pub goal_id: Option<String>,
    pub title: String,
    pub detail_md: Option<String>,
    pub current_summary: Option<String>,
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
            Ok(_) => return error_404(format!("project not found: {goal_id}")),
            Err(error) => return error_500(error.to_string()),
        }
    }
    let now = now_ms();
    let item = ProjectInitiativeRecord {
        summary: opencoder_store::project::summary::ProjectSummary::new(body.current_summary, now),
        people: body.people.normalized(),
        id: format!("pi-{}", ulid::Ulid::new()),
        goal_id: body.goal_id,
        title: title.to_owned(),
        detail_md: body.detail_md,

        sort: body.sort.unwrap_or(0),
        created_at: now,
        updated_at: now,
    };
    match deps.projects.create_initiative(&item).await {
        Ok(()) => Json(json!(item)).into_response(),
        Err(error) => crate::api_project_people::write_error(error),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchBody {
    #[serde(flatten)]
    pub people: opencoder_store::project::people::ProjectPeoplePatch,
    #[serde(default, deserialize_with = "crate::api_project_todos::double_option")]
    pub goal_id: Option<Option<String>>,
    pub title: Option<String>,
    pub detail_md: Option<String>,
    pub current_summary: Option<String>,
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
            Ok(_) => return error_404(format!("project not found: {goal_id}")),
            Err(error) => return error_500(error.to_string()),
        }
    }
    let change = ProjectInitiativePatch {
        current_summary: body.current_summary,
        people: body.people,
        goal_id: body.goal_id,
        title,
        detail_md: body.detail_md,
        sort: body.sort,
    };
    match deps.projects.patch_initiative(&id, &change, now_ms()).await {
        Ok(true) => Json(json!({ "ok": true })).into_response(),
        Ok(false) => error_404("initiative not found"),
        Err(error) => crate::api_project_people::write_error(error),
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
        Err(error) if error.is::<opencoder_store::project::InitiativeNotEmpty>() => {
            error_409("initiative contains TODOs")
        }
        Err(error) => crate::api_project_people::write_error(error),
    }
}

#[derive(Deserialize)]
pub struct OrderBody {
    pub expected_ids: Vec<String>,
    pub ids: Vec<String>,
}

pub async fn reorder(
    State(state): State<Arc<AppState>>,
    Path(goal_id): Path<String>,
    Json(body): Json<OrderBody>,
) -> Response {
    let deps = match require_deps(&state) {
        Ok(deps) => deps,
        Err(response) => return *response,
    };
    let ids: std::collections::HashSet<_> = body.ids.iter().collect();
    let expected: std::collections::HashSet<_> = body.expected_ids.iter().collect();
    if ids.len() != body.ids.len() || expected.len() != body.expected_ids.len() || ids != expected {
        return error_400("ids must be a permutation of expected_ids without duplicates");
    }
    match deps
        .projects
        .reorder_initiatives(&goal_id, &body.expected_ids, &body.ids, now_ms())
        .await
    {
        Ok(true) => Json(json!({ "ok": true })).into_response(),
        Ok(false) => error_404("project not found"),
        Err(error) if error.is::<opencoder_store::project::InitiativeOrderConflict>() => {
            error_409(error.to_string())
        }
        Err(error) => error_500(error.to_string()),
    }
}
