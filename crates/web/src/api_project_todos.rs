//! `/api/project/todos` CRUD. TODO status is user-managed; execution history
//! is linked by ID and inspected through the execution index.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use opencoder_core::message::now_ms;
use opencoder_store::{ProjectStore, ProjectTodoPatch, ProjectTodoRecord, ProjectTodoStatus};

use crate::api_project_util::{error_400, error_404, error_500, require_deps, to_json};
use crate::AppState;

#[derive(Deserialize)]
pub struct TodoQuery {
    pub initiative_id: Option<String>,
}

#[derive(Deserialize)]
pub struct ReorderBody {
    pub initiative_id: Option<String>,
    pub board_status: String,
    pub ids: Vec<String>,
}

pub async fn reorder_todos(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ReorderBody>,
) -> Response {
    if !matches!(
        body.board_status.as_str(),
        "backlog" | "todo" | "in_progress" | "done"
    ) || body.ids.is_empty()
        || body.ids.len() > 1000
    {
        return error_400("invalid board reorder");
    }
    let deps = match require_deps(&state) {
        Ok(deps) => deps,
        Err(reply) => return *reply,
    };
    {
        match deps
            .projects
            .list_todos(body.initiative_id.as_deref())
            .await
        {
            Ok(todos)
                if body.ids.iter().all(|id| {
                    todos
                        .iter()
                        .any(|todo| &todo.id == id && todo.initiative_id == body.initiative_id)
                }) => {}
            Ok(_) => return error_400("reorder contains TODOs outside this initiative"),
            Err(error) => return error_500(error.to_string()),
        }
    }
    match deps
        .projects
        .reorder_todos(
            body.initiative_id.as_deref(),
            &body.board_status,
            &body.ids,
            now_ms(),
        )
        .await
    {
        Ok(()) => Json(json!({"ok":true})).into_response(),
        Err(error) => error_500(format!("reorder TODOs: {error:#}")),
    }
}

async fn group_exists(projects: &dyn ProjectStore, id: &str) -> anyhow::Result<bool> {
    Ok(projects
        .list_initiatives(None)
        .await?
        .iter()
        .any(|item| item.id == id))
}

/// GET /api/project/todos?initiative_id= — one initiative's todos; without the
/// parameter ALL todos are listed (backlog included), `created_at` order.
pub async fn list_todos(
    State(state): State<Arc<AppState>>,
    Query(q): Query<TodoQuery>,
) -> Response {
    let deps = match require_deps(&state) {
        Ok(d) => d,
        Err(r) => return *r,
    };
    match deps.projects.list_todos(q.initiative_id.as_deref()).await {
        Ok(items) => match deps.projects.list_todo_tags().await {
            Ok(links) => Json(json!({"todos": items.into_iter().map(|todo| {
                let mut value = to_json(&todo);
                value["tag_ids"] = json!(links.iter().filter(|link| link.todo_id == todo.id).map(|link| &link.tag_id).collect::<Vec<_>>());
                value
            }).collect::<Vec<_>>()})).into_response(),
            Err(error) => crate::api_project_tags::tag_error(error),
        },
        Err(e) => error_500(format!("list todos: {e:#}")),
    }
}

#[derive(Deserialize)]
pub struct CreateTodoBody {
    #[serde(default)]
    pub tag_ids: Vec<String>,
    pub board_status: Option<String>,
    /// Absent ⇒ initiative-less backlog item.
    #[serde(default)]
    pub initiative_id: Option<String>,
    pub title: String,
    pub draft: String,
    /// Optional capability selected for later explicit assignment.
    #[serde(default)]
    pub capability_id: Option<String>,
    /// Executor agent; defaults to `act`.
    #[serde(default)]
    pub agent: Option<String>,
    /// Executor dimension (P4): `agent|team|dag|brain`; absent ⇒ `agent`.
    #[serde(default)]
    pub executor_kind: Option<String>,
    /// Executor target: team/dag resource name or pinned brain capability
    /// id. Trimmed; empty ⇒ None.
    #[serde(default)]
    pub executor_ref: Option<String>,
    /// Inline executor definition (team spec / DagSpec / brain routes).
    /// Doubled so a later PATCH can distinguish null (clear) from absent.
    #[serde(default, deserialize_with = "double_option")]
    pub executor_spec: Option<Option<String>>,
}

/// POST /api/project/todos — new `draft` todo; unknown parent group → 404.
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
    let capability_id = match validate_capability(body.capability_id.as_deref()) {
        Ok(value) => value,
        Err(reply) => return reply,
    };
    if let Some(mid) = &body.initiative_id {
        match group_exists(deps.projects.as_ref(), mid).await {
            Ok(true) => {}
            Ok(false) => return error_404(format!("TODO group not found: {mid}")),
            Err(e) => return error_500(format!("verify initiative: {e:#}")),
        }
    }
    let board_status = body.board_status.as_deref().unwrap_or("backlog");
    if !matches!(board_status, "backlog" | "todo" | "in_progress" | "done") {
        return error_400("unsupported board status");
    }
    let now = now_ms();
    // Executor triple: kind string resolves (unknown → 400), spec validates
    // before anything is persisted; ref is trimmed (empty ⇒ None).
    let executor_spec = flatten_spec(body.executor_spec);
    let executor_kind =
        match validate_executor(body.executor_kind.as_deref(), executor_spec.as_deref()) {
            Ok(kind) => kind,
            Err(r) => return r,
        };
    let rec = ProjectTodoRecord {
        id: format!("pt-{}", ulid::Ulid::new()),
        initiative_id: body.initiative_id,
        title,
        draft: body.draft,
        plan_md: None,
        status: ProjectTodoStatus::Draft,
        board_status: board_status.into(),
        position: now,
        capability_id,
        agent: body.agent.unwrap_or_else(|| "act".into()),
        executor_kind,
        executor_ref: normalize_ref(body.executor_ref.as_deref()),
        executor_spec,
        active_session_id: None,
        created_at: now,
        updated_at: now,
    };
    match deps.projects.create_todo_tagged(&rec, &body.tag_ids).await {
        Ok(()) => Json(to_json(&rec)).into_response(),
        Err(e) => crate::api_project_tags::tag_error(e),
    }
}

#[derive(Deserialize)]
pub struct PatchTodoBody {
    pub tag_ids: Option<Vec<String>>,
    /// `Option<Option<String>>` + [`double_option`] distinguishes the three
    /// PATCH cases: absent ⇒ unchanged, JSON `null` ⇒ clear to the backlog,
    /// a value ⇒ re-parent (unknown initiative → 404). Plain
    /// `Option<Option<T>>` is NOT enough: serde resolves JSON `null` to the
    /// OUTER `None`, making null and absent indistinguishable.
    #[serde(default, deserialize_with = "double_option")]
    pub initiative_id: Option<Option<String>>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub draft: Option<String>,
    #[serde(default)]
    pub status: Option<ProjectTodoStatus>,
    pub board_status: Option<String>,
    pub position: Option<i64>,
    #[serde(default, deserialize_with = "double_option")]
    pub capability_id: Option<Option<String>>,
    #[serde(default)]
    pub agent: Option<String>,
    /// Executor triple, same triple semantics (absent / null-clear / value).
    /// A spec (or a null-clear under a spec-bearing kind) validates against
    /// the body's kind when given, else the CURRENT todo kind.
    #[serde(default)]
    pub executor_kind: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub executor_ref: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub executor_spec: Option<Option<String>>,
}

pub use validation::{
    double_option, flatten_spec, normalize_ref, validate_capability, validate_executor,
};
#[path = "api_project_todos/validation.rs"]
mod validation;

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
    let capability_id = match body.capability_id.as_ref() {
        None => None,
        Some(value) => match validate_capability(value.as_deref()) {
            Ok(value) => Some(value),
            Err(reply) => return reply,
        },
    };
    if body.status.is_some_and(|status| {
        !matches!(
            status,
            ProjectTodoStatus::Draft | ProjectTodoStatus::Planned | ProjectTodoStatus::Done
        )
    }) {
        return error_400("unsupported manual todo status");
    }
    if body
        .board_status
        .as_deref()
        .is_some_and(|status| !matches!(status, "backlog" | "todo" | "in_progress" | "done"))
    {
        return error_400("unsupported board status");
    }
    if body.position.is_some_and(|position| position < 0) {
        return error_400("todo position must not be negative");
    }
    if let Some(Some(mid)) = &body.initiative_id {
        match group_exists(deps.projects.as_ref(), mid).await {
            Ok(true) => {}
            Ok(false) => return error_404(format!("TODO group not found: {mid}")),
            Err(e) => return error_500(format!("verify initiative: {e:#}")),
        }
    }
    // Executor columns: null-clear vs value semantics ride the patch's
    // Option<Option<T>>; the EFFECTIVE spec — the patched one when the body
    // carries executor_spec, else the STORED one — validates against the
    // body kind else the CURRENT todo kind, so a kind-only patch
    // revalidates the stored spec against the new kind (fail-closed: no
    // stale-spec smuggling; validate_executor already rejects agent+spec,
    // so switching to agent without clearing a stored spec 400s).
    let mut executor_kind = None;
    let mut executor_ref = None;
    let mut executor_spec = None;
    if body.executor_kind.is_some() || body.executor_ref.is_some() || body.executor_spec.is_some() {
        let current = match deps.projects.get_todo(&id).await {
            Ok(Some(rec)) => rec,
            Ok(None) => return error_404(format!("todo not found: {id}")),
            Err(e) => return error_500(format!("load todo: {e:#}")),
        };
        let spec = if body.executor_spec.is_some() {
            flatten_spec(body.executor_spec.clone())
        } else {
            current.executor_spec.clone()
        };
        let kind = match validate_executor(
            body.executor_kind
                .as_deref()
                .or(Some(current.executor_kind.as_str())),
            spec.as_deref(),
        ) {
            Ok(kind) => kind,
            Err(r) => return r,
        };
        if body.executor_kind.is_some() {
            executor_kind = Some(kind);
        }
        if let Some(raw) = body.executor_ref {
            executor_ref = Some(normalize_ref(raw.as_deref()));
        }
        if body.executor_spec.is_some() {
            executor_spec = Some(spec);
        }
    }
    let patch = ProjectTodoPatch {
        title,
        draft: body.draft,
        // Old project-run snapshots are not writable through TODO CRUD.
        plan_md: None,
        status: body.status,
        board_status: body.board_status,
        position: body.position,
        capability_id,
        agent: body.agent,
        executor_kind,
        executor_ref,
        executor_spec,
        initiative_id: body.initiative_id,
        active_session_id: None,
    };
    match deps
        .projects
        .patch_todo_tagged(&id, &patch, body.tag_ids.as_deref(), now_ms())
        .await
    {
        Ok(true) => Json(json!({ "ok": true })).into_response(),
        Ok(false) => error_404(format!("todo not found: {id}")),
        Err(e) => crate::api_project_tags::tag_error(e),
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
