//! Shared helpers for the `/api/project/*` REST surface: the uniform
//! `{"ok": false, "error"}` error responses re-exported from [`crate::api`]
//! (same shape as `api_envs` / `api_subagents`), the uninitialized-service
//! 503 gate, record serialization and the plan/execute error→status mapping.

use std::sync::Arc;

use axum::response::Response;
use opencoder_project::Deps;
use serde_json::Value;

use crate::api::error_503;
pub(crate) use crate::api::{error_400, error_404, error_409, error_500};
use crate::AppState;

/// Every `/api/project` handler starts here. An uninitialized
/// `ProjectService` (an `AppState` built without `init`) answers 503 so the
/// SPA can render "project module disabled" instead of a raw 500. The error
/// is boxed: `Response` is large enough to trip `result_large_err`.
pub fn require_deps(state: &AppState) -> Result<Arc<Deps>, Box<Response>> {
    state
        .project
        .require()
        .map_err(|e| Box::new(error_503(format!("{e:#}"))))
}

/// Store records are `Serialize` with snake_case wire forms; serialize
/// directly into responses. A failure here is a record-definition bug, not a
/// runtime condition — degrade to JSON null rather than panic.
pub fn to_json<T: serde::Serialize>(rec: T) -> Value {
    serde_json::to_value(rec).unwrap_or(Value::Null)
}

/// Serialize a record list for the `{"goals": [...]}`-style responses.
pub fn rec_list<T: serde::Serialize>(recs: impl IntoIterator<Item = T>) -> Vec<Value> {
    recs.into_iter().map(to_json).collect()
}

/// `start_plan` / `start_execute` error → HTTP mapping: unknown todo → 404,
/// run-state conflicts (already running, no plan yet) → 409, uninitialized
/// service → 503, anything else → 500.
pub fn map_start_err(e: anyhow::Error) -> Response {
    let msg = format!("{e:#}");
    if msg.contains("not found") {
        error_404(msg)
    } else if msg.contains("is running")
        || msg.contains("no plan")
        || msg.contains("already accepted")
    {
        error_409(msg)
    } else if msg.contains("invalid project run id") {
        error_400(msg)
    } else if msg.contains("not initialized") || msg.contains("persistence unavailable") {
        error_503(msg)
    } else {
        error_500(format!("start: {msg}"))
    }
}
