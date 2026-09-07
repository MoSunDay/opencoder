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
use opencoder_store::{
    ProjectExecutorKind, ProjectTodoPatch, ProjectTodoRecord, ProjectTodoStatus,
};

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
        milestone_id: body.milestone_id,
        title,
        draft: body.draft,
        plan_md: None,
        status: ProjectTodoStatus::Draft,
        agent: body.agent.unwrap_or_else(|| "act".into()),
        executor_kind,
        executor_ref: normalize_ref(body.executor_ref.as_deref()),
        executor_spec,
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

/// Force deserialization of the INNER `Option<T>` so JSON `null` produces
/// `Some(None)` (clear) instead of collapsing to the outer `None` (absent).
fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Ok(Some(Option::<T>::deserialize(de)?))
}

/// Flatten a doubled spec field into the plain value that lands in the
/// record: absent/null/blank ⇒ None, else the trimmed JSON text. Blank
/// normalizes to a clear so `{"executor_spec": ""}` cannot store junk.
fn flatten_spec(spec: Option<Option<String>>) -> Option<String> {
    spec.flatten()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Trimmed executor ref; empty ⇒ None (agent falls back to `act`, team/dag
/// resolve lazily from their inline spec at execute time).
fn normalize_ref(raw: Option<&str>) -> Option<String> {
    let trimmed = raw.unwrap_or("").trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Parse `executor_kind` (absent ⇒ default agent; unknown string → 400
/// `unknown executor_kind: {x}`) and validate the non-blank inline spec
/// against it via the store-side pure `validate_spec` (the canonical
/// validator shared with the control plane; it also owns the agent-with-
/// spec rejection, "agent executor takes no spec").
// `Response` (axum) is inherently large; boxing would ripple through every
// handler call site for no gain.
#[allow(clippy::result_large_err)]
fn validate_executor(
    kind: Option<&str>,
    spec: Option<&str>,
) -> Result<ProjectExecutorKind, Response> {
    let kind = match kind {
        None => ProjectExecutorKind::default(),
        Some(raw) => ProjectExecutorKind::parse(raw)
            .ok_or_else(|| error_400(format!("unknown executor_kind: {raw}")))?,
    };
    if let Some(spec) = spec.map(str::trim).filter(|s| !s.is_empty()) {
        if let Err(e) = opencoder_store::project_executor_spec::validate_spec(kind, spec) {
            return Err(error_400(format!("executor_spec: {e:#}")));
        }
    }
    Ok(kind)
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
        // Service-owned on purpose (see module doc).
        plan_md: None,
        status: None,
        agent: body.agent,
        executor_kind,
        executor_ref,
        executor_spec,
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
