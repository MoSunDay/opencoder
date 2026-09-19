//! `/api/agents` — REST surface for file-based custom agents
//! (`~/.opencoder/agents/`): reference cards. Reads go through
//! `opencoder_core::agent`, writes through `opencoder_agents`; the
//! shared, versioned resource pools the cards reference live in
//! [`crate::api_agent_resources`]. Every card write fans
//! `DrainCmd::ReloadConfig` out to live sessions (mirrors `PATCH
//! /api/config` and `/api/envs`) so live pools stay fresh; the sessions'
//! own agent stays whatever it was scheduled with (会话级 agent 切换走
//! `/api/sessions/:id/agent`).

#[path = "api_agents/resources.rs"]
pub mod resources;

use std::io;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use opencoder_agents::{
    delete_agent,
    write::{create_agent_with_profile, update_agent_with_profile},
};
use opencoder_core::agent::{list_agents, read_agent_meta, validate_agent_name, AgentRefs};

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
    state.reload_agents().await;
}

/// GET /api/agents — registered cards only (agents root), sorted by name.
/// Builtin scheduling roles (`act`/`plan`/`command`)
/// stay in the runtime (`opencoder_core::agent::builtin_agents`) and are NOT
/// listed here: this endpoint is the management surface for file cards, not
/// a union with runtime roles. SPA consumers that offer "pick a primary
/// agent" merge the builtin trio client-side (`spa/src/agents/builtins.js`).
pub async fn list(State(_state): State<Arc<AppState>>) -> Response {
    let names: std::collections::BTreeSet<String> = list_agents().into_iter().collect();
    let agents: Vec<Value> = names
        .into_iter()
        .filter_map(|name| {
            let meta = visible_meta(&name)?;
            // One-line identity for pickers: the card prompt pool's soul.md
            // first non-empty line (`meta::agent_description`), else the same
            // generic label `resolve_file_agent` uses — SPA `@`/`/agent` and
            // TUI `/agent` menus render this string.
            let description = opencoder_core::agent::agent_description(&name)
                .unwrap_or_else(|| format!("Custom agent {name}"));
            Some(json!({
                "name": name,
                "primary": opencoder_core::resolve_agent(&name).is_some_and(|a| a.is_primary() && a.name != "workflow"),
                "builtin": opencoder_core::builtin_agents().iter().any(|a| a.name == name),
                "description": description,
                "harness": meta.harness,
                "harness_profile": meta.harness_profile,
                "current": meta.current,
                "references": opencoder_agents::references::references_snapshot(&meta),
                "updated_at": meta.updated_at,
            }))
        })
        .collect();
    Json(json!({ "ok": true, "agents": agents })).into_response()
}

#[derive(Deserialize)]
pub struct CreateBody {
    pub harness_profile: Option<String>,
    pub name: String,
    #[serde(default)]
    pub harness: opencoder_core::harness::Harness,
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
    match create_agent_with_profile(&name, body.current, body.harness, body.harness_profile) {
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
    match visible_meta(&name) {
        Some(mut meta) => {
            meta.references = opencoder_agents::references::references_snapshot(&meta);
            Json(json!({ "ok": true, "meta": meta, "builtin": opencoder_core::builtin_agents().iter().any(|a| a.name == name) })).into_response()
        }
        None => error_404(&format!("unknown agent: {name}")),
    }
}

#[derive(Deserialize)]
pub struct UpdateBody {
    #[serde(default, deserialize_with = "profile_update")]
    pub harness_profile: Option<Option<String>>,
    pub current: Option<AgentRefs>,
    pub harness: Option<opencoder_core::harness::Harness>,
}

fn profile_update<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(deserializer).map(Some)
}

/// PUT /api/agents/:name — rewrite the card's references (one history
/// entry per changed field, `references` snapshot refreshed). ReloadConfig
/// fans out unconditionally: a live session's resolved pool snapshot can
/// change even when the card is not its scheduled agent (会话级切换与卡片
/// 写已解耦，快照刷新不依赖激活判断).
pub async fn update(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<UpdateBody>,
) -> Response {
    match update_agent_with_profile(&name, body.current, body.harness, body.harness_profile) {
        Ok(()) => {
            fan_out_reload(&state).await;
            Json(json!({ "ok": true })).into_response()
        }
        Err(e) => io_error_response("update agent", e),
    }
}

/// DELETE /api/agents/:name — drop the card (resource pools are shared and
/// never touched). Missing card ⇒ 404. ReloadConfig fans out
/// unconditionally so live sessions drop their stale snapshot.
pub async fn delete(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    if opencoder_core::builtin_agents()
        .iter()
        .any(|a| a.name == name)
    {
        return error_400("cannot delete a builtin agent".into());
    }
    if read_agent_meta(&name).is_none() {
        return error_404(&format!("unknown agent: {name}"));
    }
    match delete_agent(&name) {
        Ok(()) => {
            fan_out_reload(&state).await;
            Json(json!({ "ok": true, "deleted": name })).into_response()
        }
        Err(e) => io_error_response("delete agent", e),
    }
}

fn visible_meta(name: &str) -> Option<opencoder_core::agent::AgentMeta> {
    read_agent_meta(name).or_else(|| {
        opencoder_core::builtin_agents()
            .into_iter()
            .find(|a| a.name == name)
            .map(|_| opencoder_core::agent::AgentMeta {
                name: name.into(),
                ..Default::default()
            })
    })
}
