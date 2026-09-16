//! `/api/schedules` — inspect the control-plane cron scheduler.
//!
//! `schedules.json` (a domain file next to the server config) is the
//! definition source of truth; the DB only carries the fire ledger. Both
//! handlers therefore read: definitions from the working dir (fail-soft
//! load) and history from the Store. The admin-only role gate applies
//! (unknown paths default to closed).

use super::{error_400, response};
use crate::AppState;
use axum::{
    extract::{Path, Query, State},
    response::Response,
    routing::get,
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
        .route("/api/schedules", get(list))
        .route("/api/schedules/:id/runs", get(runs))
}

/// All configured schedules with their latest fire and the next tick.
async fn list(State(state): State<Arc<AppState>>) -> Response {
    let config = load_schedules(&state.workdir);
    let now_ms = now_ms();
    let mut schedules = Vec::new();
    for job in &config.schedules {
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
            "last_run": last_run,
            "next_run": next_run,
        }));
    }
    response(super::RpcReply::ok(json!({
        "schedules": schedules,
        "scan_interval_secs": config.scan_interval_secs,
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
        Ok(runs) => response(super::RpcReply::ok(json!({ "runs": runs }))),
        Err(e) => response(super::RpcReply::error(500, e.to_string())),
    }
}
