//! `/api/agents` — REST surface for file-based custom agents
//! (`~/.opencoder/agents/`): reference cards + the active marker. Reads go
//! through `opencode_core::agent`, writes through `opencode_agents`; the
//! shared, versioned resource pools the cards reference live in
//! [`crate::api_agent_resources`]. Anything that can change the ACTIVE
//! agent's chain fans `DrainCmd::ReloadConfig` out to live sessions
//! (mirrors `PATCH /api/config` and `/api/envs`); writes that cannot touch
//! a live session stay silent disk writes.

use std::io;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use opencode_agents::{create_agent, delete_agent, update_agent_refs};
use opencoder_core::agent::{
    active_agent, list_agents, read_agent_meta, resource_current_version_dir, set_active_agent,
    set_active_agent_checked, validate_agent_name, AgentRefs,
};

/// Serializes activation writes (PATCH /api/agents/active): the marker
/// swap itself is atomic, but two concurrent PATCHes could otherwise
/// interleave check-then-act (both see "not active", both flip, both fan
/// out) — the in-process gate keeps activation ordered. Mirrors
/// `ACTIVATE_GATE` in `api_envs`.
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

/// Map write-path io errors onto REST statuses the same way the envs
/// layer does: `NotFound` ⇒ 404 (unknown card), `AlreadyExists` ⇒ 409
/// (duplicate), `InvalidInput`/`InvalidData` (validation, preflight
/// rollback) ⇒ 400, anything else ⇒ 500.
fn io_error_response(ctx: &str, e: io::Error) -> Response {
    match e.kind() {
        io::ErrorKind::NotFound => error_404(&format!("{ctx}: {e}")),
        io::ErrorKind::AlreadyExists => error_409(&format!("{ctx}: {e}")),
        io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => {
            error_400(format!("{ctx}: {e}"))
        }
        _ => error_500(format!("{ctx}: {e}")),
    }
}

/// Fan `ReloadConfig` out to every live session handle (same mechanism as
/// `PATCH /api/config` and `/api/envs`): snapshot ids under the lock, then
/// send unlocked. Shared with the resource endpoints.
pub(crate) async fn fan_out_reload(state: &AppState) {
    let session_ids: Vec<String> = {
        let map = state.handles.lock().await;
        map.keys().cloned().collect()
    };
    for sid in &session_ids {
        send_cmd(&state.handles, sid, DrainCmd::ReloadConfig).await;
    }
}

/// Whether the ACTIVE agent's `current` card references the pool resource
/// `cat`/`resource` — the only resource writes that can change a live
/// session's chain. Shared with the resource endpoints.
pub(crate) fn active_chain_references(cat: &str, resource: &str) -> bool {
    let Some(card) = active_agent().and_then(|name| read_agent_meta(&name)) else {
        return false;
    };
    let field = match cat {
        "prompts" => card.current.prompt,
        "skills" => card.current.skills,
        "tools" => card.current.tools,
        "memory" => card.current.memory,
        _ => None,
    };
    field.as_deref() == Some(resource)
}

/// GET /api/agents — every card plus which one is active (sorted by name).
pub async fn list(State(_state): State<Arc<AppState>>) -> Response {
    let active = active_agent();
    let agents: Vec<Value> = list_agents()
        .into_iter()
        .filter_map(|name| {
            let meta = read_agent_meta(&name)?;
            Some(json!({
                "name": name,
                "current": meta.current,
                "references": meta.references,
                "updated_at": meta.updated_at,
            }))
        })
        .collect();
    Json(json!({ "ok": true, "active": active, "agents": agents })).into_response()
}

#[derive(Deserialize)]
pub struct CreateBody {
    pub name: String,
    /// Initial references (all optional; empty card when omitted).
    #[serde(default)]
    pub current: AgentRefs,
}

/// POST /api/agents — create a reference card. 400 invalid name, 409
/// duplicate.
pub async fn create(State(_state): State<Arc<AppState>>, Json(body): Json<CreateBody>) -> Response {
    let name = body.name.trim().to_string();
    if let Err(e) = validate_agent_name(&name) {
        return error_400(format!("invalid agent name: {e}"));
    }
    match create_agent(&name, body.current) {
        Ok(()) => (
            StatusCode::CREATED,
            Json(json!({ "ok": true, "name": name })),
        )
            .into_response(),
        Err(e) => io_error_response("create agent", e),
    }
}

/// GET /api/agents/:name/meta — the full card (history included).
pub async fn meta(State(_state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    match read_agent_meta(&name) {
        Some(meta) => Json(json!({ "ok": true, "meta": meta })).into_response(),
        None => error_404(&format!("unknown agent: {name}")),
    }
}

#[derive(Deserialize)]
pub struct UpdateBody {
    pub current: AgentRefs,
}

/// PUT /api/agents/:name — rewrite the card's references (one history
/// entry per changed field, `references` snapshot refreshed). ReloadConfig
/// fans out only when the ACTIVE card changed — a non-active card cannot
/// affect any live session.
pub async fn update(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<UpdateBody>,
) -> Response {
    let was_active = active_agent().as_deref() == Some(name.as_str());
    match update_agent_refs(&name, body.current) {
        Ok(()) => {
            if was_active {
                fan_out_reload(&state).await;
            }
            Json(json!({ "ok": true })).into_response()
        }
        Err(e) => io_error_response("update agent", e),
    }
}

/// DELETE /api/agents/:name — drop the card (the active marker is cleared
/// first when this card holds it; resource pools are shared and never
/// touched). Missing card ⇒ 404. ReloadConfig fans out when the ACTIVE
/// card was deleted.
pub async fn delete(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    if read_agent_meta(&name).is_none() {
        return error_404(&format!("unknown agent: {name}"));
    }
    let was_active = active_agent().as_deref() == Some(name.as_str());
    if was_active {
        if let Err(e) = set_active_agent(None) {
            return error_500(format!("clear active marker: {e}"));
        }
    }
    match delete_agent(&name) {
        Ok(()) => {
            if was_active {
                fan_out_reload(&state).await;
            }
            Json(json!({ "ok": true, "deleted": name })).into_response()
        }
        Err(e) => io_error_response("delete agent", e),
    }
}

#[derive(Deserialize)]
pub struct PatchActiveBody {
    /// `Some(name)` activates; `null` deactivates.
    pub active: Option<String>,
}

/// PATCH /api/agents/active — activate (`{"active": name}`) or deactivate
/// (`{"active": null}`). The preflight parses the card and resolves its
/// `current.prompt` pool reference to a live version dir; a card with NO
/// prompt reference is rejected before the marker settles (it would
/// resolve to `None` and reads would silently fall back to `act`);
/// `set_active_agent_checked` rolls the marker back when it fails.
/// ReloadConfig fans out only when the value actually changed.
pub async fn patch_active(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PatchActiveBody>,
) -> Response {
    let target = match body.active.as_deref().map(str::trim) {
        None => None,
        Some("") => {
            return error_400(
                "active agent name cannot be blank; send `null` to deactivate".to_string(),
            )
        }
        Some(name) => Some(name.to_string()),
    };
    let _gate = ACTIVATE_GATE.lock().await;
    if target == active_agent() {
        return Json(json!({ "ok": true, "active": target, "unchanged": true })).into_response();
    }
    if let Some(name) = &target {
        if read_agent_meta(name).is_none() {
            return error_404(&format!("unknown agent: {name}"));
        }
    }
    // Preflight: card parses (again, atomically) and its prompt reference
    // is present and resolves to a pool version dir — activating a broken
    // chain OR a promptless card (which reads would silently downgrade to
    // `act`) must fail BEFORE the marker settles (rollback happens inside
    // on failure).
    let preflight = || -> Result<(), String> {
        let Some(name) = &target else {
            return Ok(());
        };
        let card = read_agent_meta(name).ok_or_else(|| format!("card `{name}` unreadable"))?;
        match card.current.prompt.as_deref() {
            None => Err(format!(
                "card `{name}` has no prompt reference — not a resolvable agent (reads would silently fall back to act)"
            )),
            Some(res) => resource_current_version_dir("prompts", res)
                .map(|_| ())
                .ok_or_else(|| format!("prompt resource `{res}` has no live version")),
        }
    };
    match set_active_agent_checked(target.as_deref(), preflight) {
        Ok(()) => {
            fan_out_reload(&state).await;
            Json(json!({ "ok": true, "active": target })).into_response()
        }
        Err(e) => io_error_response("set active agent", e),
    }
}
