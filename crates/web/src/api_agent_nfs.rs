//! `/api/agents/nfs` — lifecycle control for the read-only NFSv3 export
//! of the agents root (`opencoder_agents::serve`). GET reports the live
//! snapshot; POST `{enabled}` starts/stops the server explicitly.
//!
//! State lives in a process-global slot (one NFS server per daemon — the
//! same "static + tokio Mutex" pattern as `ACTIVATE_GATE` in
//! [`crate::api_agents`]), which keeps `AppState` untouched: every
//! existing construction site (serve + ~30 test harnesses) stays valid
//! and this wiring stays purely additive.
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
use opencoder_agents::{
    default_opts_from_config, nfs_status, spawn_nfs_server, NfsServerHandle, NfsServerStatus,
};
use opencoder_core::Config;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::AppState;

/// The one live NFS server. `spawn_nfs_server` runs the accept loop on a
/// dedicated OS thread (independent of any runtime), so the slot only
/// owns the handle; stop must go through [`NfsServerHandle::shutdown`].
static NFS_SLOT: tokio::sync::Mutex<Option<NfsServerHandle>> = tokio::sync::Mutex::const_new(None);

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
    let slot = NFS_SLOT.lock().await;
    Json(json!({ "ok": true, "status": status_value(&nfs_status(slot.as_ref())) })).into_response()
}

#[derive(Deserialize)]
pub struct SetBody {
    pub enabled: bool,
}

/// POST /api/agents/nfs — explicit lifecycle switch. `enabled:true` is
/// idempotent (an already-running server is reused, not respawned —
/// same bound port); `enabled:false` stops and clears the slot, also
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

/// Start under the slot lock: reuse the live handle when present, else
/// spawn from config. `Ok((status, started))`; spawn failures surface as
/// an error message for the 500 path (and as a log line at autostart).
async fn start_locked(config: &Config) -> Result<(NfsServerStatus, bool), String> {
    let opts = default_opts_from_config(config);
    let mut slot = NFS_SLOT.lock().await;
    if let Some(handle) = slot.as_ref() {
        return Ok((nfs_status(Some(handle)), false));
    }
    // spawn blocks briefly (bind + handshake with the server thread) and
    // its internals are sync — keep that off the async workers while the
    // async lock is held.
    let spawned = tokio::task::spawn_blocking(move || spawn_nfs_server(&opts))
        .await
        .map_err(|e| format!("nfs spawn task: {e}"))?
        .map_err(|e| format!("nfs spawn: {e:#}"))?;
    let status = nfs_status(Some(&spawned));
    tracing::info!(
        host = %status.host,
        port = status.port,
        export_root = %status.export_root,
        "agents nfs export started"
    );
    *slot = Some(spawned);
    Ok((status, true))
}

/// Stop and clear the slot (idempotent). The handle is taken out first so
/// concurrent starts never observe a half-shut server; `shutdown` parks
/// up to its bounded timeout, hence `spawn_blocking`.
async fn stop() -> Response {
    let handle = NFS_SLOT.lock().await.take();
    if let Some(handle) = handle {
        let _ = tokio::task::spawn_blocking(move || handle.shutdown()).await;
        tracing::info!("agents nfs export stopped");
    }
    Json(json!({
        "ok": true,
        "status": status_value(&nfs_status(None)),
        "started": false,
    }))
    .into_response()
}

/// Daemon autostart seam, called from `serve` before the HTTP listener
/// binds: when `agent.nfs.enabled` is set, bring the export up so it is
/// live by the time the API answers. Failure is logged and swallowed —
/// a broken export must never take the daemon down (`GET /api/agents/nfs`
/// will simply report stopped).
pub(crate) async fn autostart(workdir: &Path) {
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
