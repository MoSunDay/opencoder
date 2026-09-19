//! Admin write surface for the schedule definitions (schema v27): create,
//! full update, enable/disable, delete, and a manual immediate fire.
//!
//! Contract: every stored definition is structurally valid
//! (`ScheduleJob::validate` is the single gate — the scheduler may assume
//! it), `id` is the唯一 key (missing on create → auto ULID, duplicate →
//! 409), and deletion keeps the `schedule_runs` ledger (no FK — the audit
//! trail outlives the definition).

use super::{error_400, error_404, error_409, error_500, response};
use crate::AppState;
use axum::{
    extract::{Path, State},
    response::Response,
    Json,
};
use opencoder_core::{
    config::{validate_schedule_id, ScheduleJob},
    message::now_ms,
};
use serde_json::{json, Value};
use std::sync::Arc;

/// Parse a request body into a [`ScheduleJob`]. `id_override` pins the path
/// id (PUT/PATCH); on create a missing/blank id gets a generated one.
fn parse_job(id_override: Option<&str>, body: &Value) -> Result<ScheduleJob, String> {
    let mut value = body.clone();
    match id_override {
        Some(id) => value["id"] = json!(id),
        None => {
            let blank = value["id"].as_str().map(|s| s.trim().is_empty());
            if blank.unwrap_or(true) {
                value["id"] = json!(format!("schedule-{}", ulid::Ulid::new()));
            }
        }
    }
    serde_json::from_value::<ScheduleJob>(value).map_err(|e| format!("invalid schedule body: {e}"))
}

/// Validated body of the enable/disable patch.
#[derive(serde::Deserialize)]
pub struct ToggleBody {
    pub enabled: bool,
}

/// POST /api/schedules — create. 409 when the id already exists (updates go
/// through PUT); 400 on a structurally invalid job.
pub async fn create(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> Response {
    let job = match parse_job(None, &body) {
        Ok(job) => job,
        Err(msg) => return error_400(msg),
    };
    if let Err(msg) = job.validate() {
        return error_400(msg);
    }
    match state.store.get_schedule(&job.id).await {
        Ok(Some(_)) => {
            return error_409(&format!(
                "schedule {} already exists (use PUT to update)",
                job.id
            ))
        }
        Ok(None) => {}
        Err(e) => return error_500(e.to_string()),
    }
    match state.store.upsert_schedule(&job, now_ms()).await {
        Ok(()) => response(super::RpcReply::ok(json!({"ok": true, "id": job.id}))),
        Err(e) => error_500(e.to_string()),
    }
}

/// PUT /api/schedules/:id — full update of an existing definition. The path
/// id wins over any body id; `created_at` is preserved by the store's
/// upsert, `updated_at` moves to now. 404 when the id is unknown.
pub async fn update(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    if validate_schedule_id(&id).is_err() {
        return error_400("invalid schedule id".into());
    }
    let job = match parse_job(Some(&id), &body) {
        Ok(job) => job,
        Err(msg) => return error_400(msg),
    };
    if let Err(msg) = job.validate() {
        return error_400(msg);
    }
    match state.store.get_schedule(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return error_404(&format!("schedule {id} not found")),
        Err(e) => return error_500(e.to_string()),
    }
    match state.store.upsert_schedule(&job, now_ms()).await {
        Ok(()) => response(super::RpcReply::ok(json!({"ok": true, "id": job.id}))),
        Err(e) => error_500(e.to_string()),
    }
}

/// PATCH /api/schedules/:id — enable/disable only (`{"enabled": bool}`).
/// Re-validates the whole job so enabling a parked-with-broken-cron
/// definition fails at the door instead of 3am.
pub async fn toggle(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<ToggleBody>,
) -> Response {
    let mut def = match state.store.get_schedule(&id).await {
        Ok(Some(def)) => def,
        Ok(None) => return error_404(&format!("schedule {id} not found")),
        Err(e) => return error_500(e.to_string()),
    };
    def.job.enabled = body.enabled;
    if let Err(msg) = def.job.validate() {
        return error_400(msg);
    }
    match state.store.upsert_schedule(&def.job, now_ms()).await {
        Ok(()) => response(super::RpcReply::ok(json!({"ok": true, "id": id}))),
        Err(e) => error_500(e.to_string()),
    }
}

/// DELETE /api/schedules/:id — remove the definition; the fire history
/// stays queryable (`/runs`). 404 when the id is unknown.
pub async fn remove(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.store.get_schedule(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return error_404(&format!("schedule {id} not found")),
        Err(e) => return error_500(e.to_string()),
    }
    match state.store.delete_schedule(&id).await {
        Ok(()) => response(super::RpcReply::ok(json!({"ok": true}))),
        Err(e) => error_500(e.to_string()),
    }
}

/// POST /api/schedules/:id/run — manual immediate fire. Submits one tick
/// with `scheduled_for_ms = now` through the same path the cron loop uses
/// (same deterministic execution id shape, same ledger row); bypasses
/// `enabled`/`overlap` because it is an explicit operator action.
pub async fn run(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let def = match state.store.get_schedule(&id).await {
        Ok(Some(def)) => def,
        Ok(None) => return error_404(&format!("schedule {id} not found")),
        Err(e) => return error_500(e.to_string()),
    };
    if let Err(msg) = def.job.validate() {
        return error_400(msg);
    }
    let scheduled_for_ms = now_ms();
    match crate::scheduler::fire_now(&state, &def.job).await {
        Ok(execution_id) => response(super::RpcReply::ok(json!({
            "ok": true,
            "scheduled_for_ms": scheduled_for_ms,
            "execution_id": execution_id,
        }))),
        Err(e) => error_500(e.to_string()),
    }
}
