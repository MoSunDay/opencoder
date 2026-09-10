//! `/api/agents/nfs` — lifecycle control for the read-only NFSv3 export
//! of the agents root (`opencoder_agents::serve`), one of the named
//! exports managed by [`crate::nfs_exports`] (key
//! [`nfs_exports::AGENTS_EXPORT`]). GET reports the live snapshot; POST
//! `{enabled}` starts/stops the server explicitly.
//!
//! Server state lives in that process-global registry, which keeps
//! `AppState` untouched: every existing construction site (serve + ~30
//! test harnesses) stays valid and this wiring stays purely additive.
//!
//! Config interplay: the handler re-reads `Config::load(workdir)` per
//! request for host/port/export_root, so config edits apply on the NEXT
//! start — but nothing here restarts the server on `ReloadConfig`
//! fan-out; lifecycle changes go through this endpoint (or daemon
//! autostart) only.

use std::path::Path;
use std::sync::Arc;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use opencoder_agents::{default_opts_from_config, NfsServerStatus};
use opencoder_core::Config;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::nfs_exports::{self, AGENTS_EXPORT};
use crate::AppState;

fn error_500(msg: String) -> Response {
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

fn status_value(status: &NfsServerStatus) -> Value {
    serde_json::to_value(status).unwrap_or_else(|_| json!({}))
}

/// GET /api/agents/nfs — current snapshot; stopped defaults when no
/// server is running.
pub async fn get_status(State(_state): State<Arc<AppState>>) -> Response {
    Json(json!({ "ok": true, "status": status_value(&nfs_exports::status(AGENTS_EXPORT)) }))
        .into_response()
}

#[derive(Deserialize)]
pub struct SetBody {
    pub enabled: bool,
}

/// POST /api/agents/nfs — explicit lifecycle switch. `enabled:true` is
/// idempotent (an already-running server is reused, not respawned —
/// same bound port); `enabled:false` stops and clears the export, also
/// idempotent.
pub async fn post_set(State(state): State<Arc<AppState>>, Json(body): Json<SetBody>) -> Response {
    if body.enabled {
        let config = match Config::load(&state.workdir) {
            Ok(c) => c,
            Err(e) => return error_500(format!("config: {e:#}")),
        };
        match start_locked(&config).await {
            Ok((status, started)) => {
                Json(json!({ "ok": true, "status": status_value(&status), "started": started }))
                    .into_response()
            }
            Err(e) => error_500(e),
        }
    } else {
        stop().await
    }
}

/// Start the agents export: reuse the live handle when present, else
/// spawn from config. `Ok((status, started))`; spawn failures surface as
/// an error message for the 500 path (and as a log line at autostart).
pub async fn start_locked(config: &Config) -> Result<(NfsServerStatus, bool), String> {
    nfs_exports::start(AGENTS_EXPORT, default_opts_from_config(config)).await
}

/// Stop and clear the agents export (idempotent). The handle is taken
/// out of the registry first so concurrent starts never observe a
/// half-shut server.
async fn stop() -> Response {
    nfs_exports::stop(AGENTS_EXPORT).await;
    Json(json!({
        "ok": true,
        "status": status_value(&nfs_exports::status(AGENTS_EXPORT)),
        "started": false,
    }))
    .into_response()
}

/// Daemon autostart seam, called from `serve` before the HTTP listener
/// binds: when `agent.nfs.enabled` is set, bring the export up so it is
/// live by the time the API answers. Failure is logged and swallowed —
/// a broken export must never take the daemon down (`GET /api/agents/nfs`
/// will simply report stopped).
pub async fn autostart(workdir: &Path) {
    let config = match Config::load(workdir) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("agents nfs autostart skipped, config load failed: {e:#}");
            return;
        }
    };
    if !config.agent.nfs.enabled {
        return;
    }
    if let Err(e) = start_locked(&config).await {
        tracing::warn!("agents nfs autostart failed (continuing): {e}");
    }
}
