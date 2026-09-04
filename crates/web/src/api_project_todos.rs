//! `/api/project/todos` CRUD — backlog and milestone todos. `status` and
//! `plan_md` are deliberately NOT patchable here: the todo state machine
//! (`draft → planned → running → done|failed`) is owned by the project
//! service's plan/execute runs (see [`crate::api_project_runs`]).

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use opencoder_core::message::now_ms;
use opencoder_store::{ProjectTodoPatch, ProjectTodoRecord, ProjectTodoStatus};

use crate::api_project_util::{error_400, error_404, error_500, rec_list, require_deps, to_json};
use crate::AppState;

#[derive(Deserialize)]
pub struct TodoQuery {
    pub milestone_id: Option<String>,
}

/// GET /api/project/todos?milestone_id= — one milestone's todos; without the
/// parameter ALL todos are listed (backlog included), `created_at` order.
pub async fn list_todos(
    State(state): State<Arc<AppState>>,
    Query(q): Query<TodoQuery>,
) -> Response {
    let deps = match require_deps(&state) {
        Ok(d) => d,
        Err(r) => return *r,
    };
    match deps.projects.list_todos(q.milestone_id.as_deref()).await {
        Ok(items) => Json(json!({ "todos": rec_list(items) })).into_response(),
        Err(e) => error_500(format!("list todos: {e:#}")),
    }
}

#[derive(Deserialize)]
pub struct CreateTodoBody {
    /// Absent ⇒ milestone-less backlog item.
    #[serde(default)]
    pub milestone_id: Option<String>,
    pub title: String,
    pub draft: String,
    /// Executor agent; defaults to `act`.
    #[serde(default)]
    pub agent: Option<String>,
}

/// POST /api/project/todos — new `draft` todo; unknown milestone → 404.
pub async fn create_todo(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateTodoBody>,
) -> Response {
    let deps = match require_deps(&state) {
        Ok(d) => d,
        Err(r) => return *r,
    };
    let title = body.title.trim().to_string();
    if title.is_empty() {
        return error_400("todo title must not be empty");
    }
    if let Some(mid) = &body.milestone_id {
        match deps.projects.list_milestones(None).await {
            Ok(items) if items.iter().any(|m| &m.id == mid) => {}
            Ok(_) => return error_404(format!("milestone not found: {mid}")),
            Err(e) => return error_500(format!("verify milestone: {e:#}")),
        }
    }
    let now = now_ms();
    let rec = ProjectTodoRecord {
        id: format!("pt-{}", ulid::Ulid::new()),
        milestone_id: body.milestone_id,
        title,
        draft: body.draft,
        plan_md: None,
        status: ProjectTodoStatus::Draft,
        agent: body.agent.unwrap_or_else(|| "act".into()),
        active_session_id: None,
        created_at: now,
        updated_at: now,
    };
    match deps.projects.create_todo(&rec).await {
        Ok(()) => Json(to_json(&rec)).into_response(),
        Err(e) => error_500(format!("create todo: {e:#}")),
    }
}

#[derive(Deserialize)]
pub struct PatchTodoBody {
    /// `Option<Option<String>>` + [`double_option`] distinguishes the three
    /// PATCH cases: absent ⇒ unchanged, JSON `null` ⇒ clear to the backlog,
    /// a value ⇒ re-parent (unknown milestone → 404). Plain
    /// `Option<Option<T>>` is NOT enough: serde resolves JSON `null` to the
    /// OUTER `None`, making null and absent indistinguishable.
    #[serde(default, deserialize_with = "double_option")]
    pub milestone_id: Option<Option<String>>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub draft: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
}

/// Force deserialization of the INNER `Option<T>` so JSON `null` produces
/// `Some(None)` (clear) instead of collapsing to the outer `None` (absent).
fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Ok(Some(Option::<T>::deserialize(de)?))
}

/// PATCH /api/project/todos/:id — partial update; unknown id → 404.
pub async fn patch_todo(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<PatchTodoBody>,
) -> Response {
    let deps = match require_deps(&state) {
        Ok(d) => d,
        Err(r) => return *r,
    };
    let title = match body.title {
        None => None,
        Some(s) if s.trim().is_empty() => return error_400("todo title must not be empty"),
        Some(s) => Some(s.trim().to_string()),
    };
    if let Some(Some(mid)) = &body.milestone_id {
        match deps.projects.list_milestones(None).await {
            Ok(items) if items.iter().any(|m| &m.id == mid) => {}
            Ok(_) => return error_404(format!("milestone not found: {mid}")),
            Err(e) => return error_500(format!("verify milestone: {e:#}")),
        }
    }
    let patch = ProjectTodoPatch {
        title,
        draft: body.draft,
        // Service-owned on purpose (see module doc).
        plan_md: None,
        status: None,
        agent: body.agent,
        milestone_id: body.milestone_id,
        active_session_id: None,
    };
    match deps.projects.patch_todo(&id, &patch, now_ms()).await {
        Ok(true) => Json(json!({ "ok": true })).into_response(),
        Ok(false) => error_404(format!("todo not found: {id}")),
        Err(e) => error_500(format!("patch todo: {e:#}")),
    }
}

/// DELETE /api/project/todos/:id — cascades the todo's runs.
pub async fn delete_todo(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let deps = match require_deps(&state) {
        Ok(d) => d,
        Err(r) => return *r,
    };
    match deps.projects.delete_todo(&id).await {
        Ok(true) => Json(json!({ "deleted": true })).into_response(),
        Ok(false) => error_404(format!("todo not found: {id}")),
        Err(e) => error_500(format!("delete todo: {e:#}")),
    }
}
