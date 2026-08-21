//! Metadata endpoints for the web UI: requirement annotation, session
//! autopilot mode, provider/model catalog, skill catalog.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use opencoder_core::{ApMode, Config};
use opencoder_store::SessionPatch;

use crate::cmd::DrainCmd;
use crate::handle::send_cmd;
use crate::AppState;

#[derive(Deserialize, Default)]
pub struct AnnotationBody {
    #[serde(default)]
    pub text: Option<String>,
}

/// POST /api/sessions/:id/annotation — set or clear the requirement
/// annotation (TUI `/requirement` parity). Absent body / `null` / blank text
/// all mean CLEAR. Persists first, then forwards to a live drain via
/// `DrainCmd::SetAnnotation` so the in-memory session agrees with the store.
/// Returns the effective state read back from the store.
pub async fn post_annotation(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Option<Json<AnnotationBody>>,
) -> Response {
    let text = body.and_then(|Json(b)| b.text);
    let effective = text.filter(|t| !t.trim().is_empty());
    if store_missing(&state, &id).await {
        return not_found(&id);
    }
    let patch = match &effective {
        None => SessionPatch {
            clear_requirement: true,
            updated_at: Some(opencoder_core::message::now_ms()),
            ..Default::default()
        },
        Some(t) => SessionPatch {
            requirement: Some(t.clone()),
            updated_at: Some(opencoder_core::message::now_ms()),
            ..Default::default()
        },
    };
    if let Err(e) = state.store.update_session(&id, &patch).await {
        return error_500(format!("update_session: {e:#}"));
    }
    // Live-drain parity: same clear semantics as the store write.
    if live_and_draining(&state, &id).await {
        send_cmd(
            &state.handles,
            &id,
            DrainCmd::SetAnnotation(effective.clone()),
        )
        .await;
    }
    let requirement = state
        .store
        .get_session(&id)
        .await
        .ok()
        .flatten()
        .and_then(|m| m.requirement);
    Json(json!({ "ok": true, "requirement": requirement })).into_response()
}

#[derive(Deserialize, Default)]
pub struct AutopilotBody {
    #[serde(default)]
    pub mode: Option<String>,
}

/// POST /api/sessions/:id/autopilot — set or clear the session-scoped
/// autopilot override (TUI `/ap` parity). `null` mode clears the override
/// ("follow the global config") — deliberately WITHOUT a live-drain command
/// (the TUI has no clear path either; a clear takes effect on the next
/// drain/resume, since there is no DrainCmd for "un-override").
pub async fn post_autopilot(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Option<Json<AutopilotBody>>,
) -> Response {
    let mode_str = body.and_then(|Json(b)| b.mode);
    if store_missing(&state, &id).await {
        return not_found(&id);
    }
    let patch = match &mode_str {
        None => SessionPatch {
            clear_autopilot_mode: true,
            updated_at: Some(opencoder_core::message::now_ms()),
            ..Default::default()
        },
        Some(s) => match ApMode::parse(s) {
            None => {
                return error_400(format!(
                    "invalid mode {s:?}: expected one of \"off\" / \"ap\" / \"review\""
                ))
            }
            Some(mode) => SessionPatch {
                autopilot_mode: Some(mode.as_str().to_string()),
                updated_at: Some(opencoder_core::message::now_ms()),
                ..Default::default()
            },
        },
    };
    if let Err(e) = state.store.update_session(&id, &patch).await {
        return error_500(format!("update_session: {e:#}"));
    }
    if mode_str.is_some() && live_and_draining(&state, &id).await {
        // Unwrap-safe: parsing succeeded in the patch match above.
        let mode = ApMode::parse(mode_str.as_deref().unwrap()).unwrap();
        send_cmd(&state.handles, &id, DrainCmd::SetApMode(mode)).await;
    }
    let mode = state
        .store
        .get_session(&id)
        .await
        .ok()
        .flatten()
        .and_then(|m| m.autopilot_mode);
    Json(json!({ "ok": true, "mode": mode })).into_response()
}

/// GET /api/models — provider/model catalog for a dropdown of
/// "provider/model" ids. Sanitized by construction: api_key and header
/// VALUES are never serialized — only provider name, model id, base_url.
pub async fn get_models(State(state): State<Arc<AppState>>) -> Response {
    let config = match Config::load(&state.workdir) {
        Ok(c) => c,
        Err(e) => return error_500(format!("config: {e:#}")),
    };
    let mut providers = vec![json!({
        "provider": "(default)",
        "model": config.model,
        "base_url": config.provider.base_url,
    })];
    for name in sorted_provider_names(&config) {
        let cfg = &config.providers[&name];
        providers.push(json!({
            "provider": name,
            "model": cfg.model,
            "base_url": cfg.base_url,
        }));
    }
    // Flat dropdown entries: the default full id, then "name/model" for every
    // named provider that carries a default model.
    let mut models = vec![config.model.clone()];
    for name in sorted_provider_names(&config) {
        if let Some(m) = config.providers[&name].model.clone() {
            models.push(format!("{name}/{m}"));
        }
    }
    Json(json!({
        "default": config.model,
        "providers": providers,
        "models": models,
    }))
    .into_response()
}

/// GET /api/skills — discovered skills with their enabled flag. Body text is
/// deliberately omitted: the frontend only needs name + description.
pub async fn get_skills(State(state): State<Arc<AppState>>) -> Response {
    let config = match Config::load(&state.workdir) {
        Ok(c) => c,
        Err(e) => return error_500(format!("config: {e:#}")),
    };
    let skills: Vec<_> = opencoder_core::skill::discover()
        .into_iter()
        .map(|s| {
            json!({
                "name": s.name,
                "description": s.description,
                "enabled": config.skills.get(&s.name).map(|c| c.enabled).unwrap_or(false),
            })
        })
        .collect();
    Json(json!({ "skills": skills })).into_response()
}

fn sorted_provider_names(config: &Config) -> Vec<String> {
    let mut names: Vec<_> = config.providers.keys().cloned().collect();
    names.sort();
    names
}

async fn store_missing(state: &AppState, id: &str) -> bool {
    matches!(state.store.get_session(id).await, Ok(None))
}

/// Whether a live handle exists AND a drain is currently running — only then
/// is forwarding a drain command meaningful (otherwise the next resume reads
/// the persisted state we just wrote).
async fn live_and_draining(state: &AppState, id: &str) -> bool {
    let map = state.handles.lock().await;
    map.get(id)
        .is_some_and(|h| h.draining.load(Ordering::SeqCst))
}

fn not_found(id: &str) -> Response {
    (
        axum::http::StatusCode::NOT_FOUND,
        Json(json!({ "ok": false, "error": format!("session not found: {id}") })),
    )
        .into_response()
}

fn error_400(msg: String) -> Response {
    (
        axum::http::StatusCode::BAD_REQUEST,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

fn error_500(msg: String) -> Response {
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}
