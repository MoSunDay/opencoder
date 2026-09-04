//! `/api/envs` — REST parity with the TUI `/envs` modal. Env mutations run
//! through the core envs API (`~/.opencoder/envs/`); anything that can change
//! the effective config fans `DrainCmd::ReloadConfig` out to live sessions
//! (mirrors `PATCH /api/config`). `GET /api/config` reflects the active env
//! automatically because `Config::load` resolves the env layer.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use opencoder_core::config::envs::{
    active_env, create_env, delete_env, env_dir, list_envs, recapture_env, set_active_env_checked,
    validate_env_name,
};

/// Serializes activation writes (PATCH /api/envs): the marker swap itself is
/// atomic, but two concurrent PATCHes could otherwise interleave
/// check-then-act (both see "not active", both flip, both fan out) — the
/// in-process gate keeps activation ordered.
static ACTIVATE_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

use crate::cmd::DrainCmd;
use crate::handle::send_cmd;
use crate::AppState;

fn error_400(msg: String) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

fn error_404(msg: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

fn error_409(msg: &str) -> Response {
    (
        StatusCode::CONFLICT,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

fn error_500(msg: String) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

/// Fan `ReloadConfig` out to every live session handle (same mechanism as
/// `PATCH /api/config`): snapshot ids under the lock, then send unlocked.
async fn fan_out_reload(state: &AppState) {
    let session_ids: Vec<String> = {
        let map = state.handles.lock().await;
        map.keys().cloned().collect()
    };
    for sid in &session_ids {
        send_cmd(&state.handles, sid, DrainCmd::ReloadConfig).await;
    }
}

fn env_exists(name: &str) -> bool {
    list_envs().iter().any(|n| n == name)
}

/// GET /api/envs — the env list plus which one is active.
pub async fn list(State(_state): State<Arc<AppState>>) -> Response {
    let active = active_env();
    let envs: Vec<Value> = list_envs()
        .into_iter()
        .map(|name| {
            let path = env_dir(&name)
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            json!({ "name": name, "path": path })
        })
        .collect();
    Json(json!({ "ok": true, "active": active, "envs": envs })).into_response()
}

#[derive(Deserialize)]
pub struct CreateBody {
    pub name: String,
    /// Seed the new env from a base-chain capture (default true).
    #[serde(default = "default_true")]
    pub capture_current: bool,
}

fn default_true() -> bool {
    true
}

/// POST /api/envs — create an env, optionally seeded from the current base
/// chain. 400 invalid name, 409 duplicate.
pub async fn create(State(state): State<Arc<AppState>>, Json(body): Json<CreateBody>) -> Response {
    if let Err(e) = validate_env_name(body.name.trim()) {
        return error_400(format!("invalid env name: {e}"));
    }
    let name = body.name.trim().to_string();
    if env_exists(&name) {
        return error_409(&format!("env already exists: {name}"));
    }
    match create_env(&name, &state.workdir, body.capture_current) {
        Ok(dir) => Json(json!({
            "ok": true,
            "name": name,
            "path": dir.display().to_string(),
        }))
        .into_response(),
        Err(e) => error_500(format!("create env: {e:#}")),
    }
}

#[derive(Deserialize)]
pub struct PatchBody {
    /// `Some(name)` activates; `null` deactivates.
    pub active: Option<String>,
}

/// PATCH /api/envs — activate (`{"active": name}`) or deactivate
/// (`{"active": null}`). 404 unknown env, 400 blank name (explicit `null` is
/// the only way to deactivate); activation runs through the preflight
/// (`set_active_env_checked`) so a corrupt env is rejected instead of
/// poisoning the next startup. Already-active short-circuits: no marker
/// rewrite, no ReloadConfig fan-out.
pub async fn patch(State(state): State<Arc<AppState>>, Json(body): Json<PatchBody>) -> Response {
    let target = match body.active.as_deref().map(str::trim) {
        None => None,
        Some("") => {
            return error_400(
                "active env name cannot be blank; send `null` to deactivate".to_string(),
            )
        }
        Some(name) => Some(name.to_string()),
    };
    let _gate = ACTIVATE_GATE.lock().await;
    if target.is_some() && active_env().as_deref() == target.as_deref() {
        return Json(json!({ "ok": true, "active": target, "unchanged": true })).into_response();
    }
    if let Some(name) = &target {
        if !env_exists(name) {
            return error_404(&format!("unknown env: {name}"));
        }
    }
    match set_active_env_checked(target.as_deref(), &state.workdir) {
        Ok(()) => {
            fan_out_reload(&state).await;
            Json(json!({ "ok": true, "active": target })).into_response()
        }
        Err(e) => error_500(format!("set active env: {e}")),
    }
}

/// POST /api/envs/:name/recapture — re-snapshot the base chain into the env.
/// ReloadConfig fans out only when the env is active (otherwise the effective
/// config cannot have changed).
pub async fn recapture(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    if !env_exists(&name) {
        return error_404(&format!("unknown env: {name}"));
    }
    match recapture_env(&name, &state.workdir) {
        Ok(()) => {
            if active_env().as_deref() == Some(name.as_str()) {
                fan_out_reload(&state).await;
            }
            Json(json!({ "ok": true, "name": name })).into_response()
        }
        Err(e) => error_500(format!("recapture env: {e:#}")),
    }
}

/// DELETE /api/envs/:name — delete the env (clears the marker first when it
/// is active). ReloadConfig fans out when the active env was deleted.
pub async fn delete(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    if !env_exists(&name) {
        return error_404(&format!("unknown env: {name}"));
    }
    let was_active = active_env().as_deref() == Some(name.as_str());
    match delete_env(&name) {
        Ok(()) => {
            if was_active {
                fan_out_reload(&state).await;
            }
            Json(json!({ "ok": true, "deleted": name })).into_response()
        }
        Err(e) => error_500(format!("delete env: {e:#}")),
    }
}
