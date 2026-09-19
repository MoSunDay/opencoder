//! `/api/schedules` — the control-plane cron scheduler's definitions and
//! fire history.
//!
//! Since schema v27 the libsql `schedules` table is the definition source
//! of truth (admin CRUD in [`write`]); the legacy `schedules.json` domain
//! file is only a one-time bootstrap seed and keeps owning the ops knob
//! `scan_interval_secs` (reported read-only here, hot-read by the
//! scheduler loop). The fire ledger (`schedule_runs`) stays read-only.
//! The admin-only role gate applies (unknown paths default to closed).

mod write;

use super::{error_400, response, RpcReply};

pub(super) use super::{error_404, error_409, error_500};
use crate::AppState;
use axum::{
    extract::{Path, Query, State},
    response::Response,
    routing::{get, post},
    Router,
};
use opencoder_core::{
    config::load_schedules,
    message::now_ms,
    schedule::{to_utc, CronExpr},
};
use serde_json::json;
use std::sync::Arc;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/schedules", get(list).post(write::create))
        .route(
            "/api/schedules/:id",
            axum::routing::put(write::update)
                .patch(write::toggle)
                .delete(write::remove),
        )
        .route("/api/schedules/:id/run", post(write::run))
        .route("/api/schedules/:id/runs", get(runs))
}

/// All stored definitions with their latest fire and the next tick, stable
/// id order (the Store's list order). `scan_interval_secs` still comes from
/// `schedules.json` — an ops knob, not a job attribute.
async fn list(State(state): State<Arc<AppState>>) -> Response {
    let defs = match state.store.list_schedules().await {
        Ok(defs) => defs,
        Err(e) => return response(super::RpcReply::error(500, e.to_string())),
    };
    let now_ms = now_ms();
    let mut schedules = Vec::new();
    for def in &defs {
        let job = &def.job;
        let (last_run, next_run) = match CronExpr::parse(&job.cron, job.timezone.as_deref()) {
            Ok(expr) => {
                let last = state
                    .store
                    .last_schedule_run(&job.id)
                    .await
                    .ok()
                    .flatten()
                    .map(|run| json!(run));
                let next = expr
                    .next_after(to_utc(now_ms))
                    .map(|tick| tick.timestamp_millis());
                (last, next)
            }
            Err(_) => (None, None),
        };
        schedules.push(json!({
            "id": job.id,
            "cron": job.cron,
            "timezone": job.timezone,
            "enabled": job.enabled,
            "kind": job.kind.as_str(),
            "target": job.target,
            "params": job.params,
            "overlap": job.overlap,
            "node_id": job.node_id,
            "created_at": def.created_at,
            "updated_at": def.updated_at,
            "last_run": last_run,
            "next_run": next_run,
        }));
    }
    response(RpcReply::ok(json!({
        "schedules": schedules,
        "scan_interval_secs": load_schedules(&state.workdir).scan_interval_secs,
    })))
}

#[derive(serde::Deserialize, Default)]
pub struct RunsQuery {
    limit: Option<u32>,
}

/// Fire history of one schedule, newest tick first.
async fn runs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<RunsQuery>,
) -> Response {
    if opencoder_core::config::validate_schedule_id(&id).is_err() {
        return error_400("invalid schedule id".into());
    }
    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    match state.store.list_schedule_runs(&id, limit).await {
        Ok(runs) => response(RpcReply::ok(json!({ "runs": runs }))),
        Err(e) => response(super::RpcReply::error(500, e.to_string())),
    }
}
