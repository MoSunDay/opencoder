//! Additional HTTP handlers for feature-parity with the TUI:
//! fork, compact, handoff, config, skill, and background-process management.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use opencoder_core::Config;
use opencoder_llm::{ChatClient, ChatStream};
use opencoder_store::{SessionMeta, SessionPatch};

use crate::cmd::DrainCmd;
use crate::handle::{ensure_drain, send_cmd, SessionHandle};
use crate::AppState;

// ── fork ──────────────────────────────────────────────────────────────────

/// POST /api/sessions/:id/fork — clone a session (meta + messages).
pub async fn fork_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let meta = match state.store.get_session(&id).await {
        Ok(Some(m)) => m,
        Ok(None) => return error_404(&format!("session not found: {id}")),
        Err(e) => return error_500(format!("get_session: {e:#}")),
    };
    let messages = match state.store.load_messages(&id).await {
        Ok(m) => m,
        Err(e) => return error_500(format!("load_messages: {e:#}")),
    };
    let new_id = opencoder_session::runner::new_id();
    let now = opencoder_core::message::now_ms();
    let forked = SessionMeta {
        id: new_id.clone(),
        title: meta.title.as_deref().map(|t| format!("{t} (fork)")),
        agent: meta.agent.clone(),
        model: meta.model.clone(),
        workdir_hash: meta.workdir_hash.clone(),
        created_at: now,
        updated_at: now,
        summary: meta.summary.clone(),
        summary_seq: meta.summary_seq,
        handoff_seq: meta.handoff_seq,
        handoff_plan: meta.handoff_plan.clone(),
        skill: meta.skill.clone(),
        task_type: None,
    };
    if let Err(e) = state.store.create_session(&forked).await {
        return error_500(format!("create_session: {e:#}"));
    }
    if !messages.is_empty() {
        if let Err(e) = state.store.append_messages(&new_id, &messages).await {
            return error_500(format!("append_messages: {e:#}"));
        }
    }
    Json(json!({ "id": new_id })).into_response()
}

// ── compact ───────────────────────────────────────────────────────────────

/// POST /api/sessions/:id/compact — queue a manual compaction command.
pub async fn post_compact(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    if state.store.get_session(&id).await.ok().flatten().is_none() {
        return error_404(&format!("session not found: {id}"));
    }
    let config = match load_config(&state) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    let client = match build_client(&state, &config) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    // Queue the command FIRST (before spawning the drain) so it's in the
    // channel even if the drain processes instantly.
    let handle = {
        let mut map = state.handles.lock().await;
        map.entry(id.clone())
            .or_insert_with(SessionHandle::new)
            .clone()
    };
    let _ = handle.cmd_tx.send(DrainCmd::Compact);
    ensure_drain(
        state.handles.clone(),
        state.store.clone(),
        &id,
        client,
        state.workdir.clone(),
        config,
    )
    .await;
    Json(json!({ "ok": true })).into_response()
}

// ── handoff ───────────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
pub struct HandoffBody {
    #[serde(default)]
    pub extra: String,
}

/// POST /api/sessions/:id/handoff — execute a plan->act handoff.
pub async fn post_handoff(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Option<Json<HandoffBody>>,
) -> Response {
    if state.store.get_session(&id).await.ok().flatten().is_none() {
        return error_404(&format!("session not found: {id}"));
    }
    let extra = body.map(|b| b.extra.clone()).unwrap_or_default();
    let config = match load_config(&state) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    let client = match build_client(&state, &config) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    let handle = {
        let mut map = state.handles.lock().await;
        map.entry(id.clone())
            .or_insert_with(SessionHandle::new)
            .clone()
    };
    let _ = handle.cmd_tx.send(DrainCmd::Handoff { extra });
    ensure_drain(
        state.handles.clone(),
        state.store.clone(),
        &id,
        client,
        state.workdir.clone(),
        config,
    )
    .await;
    Json(json!({ "ok": true })).into_response()
}

// ── config ────────────────────────────────────────────────────────────────

/// GET /api/config — return the current on-disk config as JSON.
pub async fn get_config(State(state): State<Arc<AppState>>) -> Response {
    match Config::load(&state.workdir) {
        Ok(cfg) => {
            let val = serde_json::to_value(&cfg).unwrap_or_else(|_| json!({}));
            Json(val).into_response()
        }
        Err(e) => error_500(format!("config load: {e:#}")),
    }
}

/// PATCH /api/config — merge a JSON patch into the config file and reload.
pub async fn patch_config(
    State(state): State<Arc<AppState>>,
    Json(patch): Json<serde_json::Value>,
) -> Response {
    if let Err(e) = Config::save(&state.workdir, &patch) {
        return error_500(format!("config save: {e:#}"));
    }
    // Reload all live sessions that have a handle.
    let session_ids: Vec<String> = {
        let map = state.handles.lock().await;
        map.keys().cloned().collect()
    };
    for sid in &session_ids {
        send_cmd(&state.handles, sid, DrainCmd::ReloadConfig).await;
    }
    Json(json!({ "ok": true })).into_response()
}

// ── skill ─────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SkillBody {
    pub skill: Option<String>,
}

/// POST /api/sessions/:id/skill — set or clear the active skill.
pub async fn post_skill(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<SkillBody>,
) -> Response {
    let mut patch = SessionPatch {
        updated_at: Some(opencoder_core::message::now_ms()),
        ..Default::default()
    };
    match &body.skill {
        Some(s) => patch.skill = Some(s.clone()),
        None => patch.clear_skill = true,
    }
    if let Err(e) = state.store.update_session(&id, &patch).await {
        return error_500(format!("update_session: {e:#}"));
    }
    send_cmd(&state.handles, &id, DrainCmd::SetSkill(body.skill.clone())).await;
    Json(json!({ "ok": true, "skill": body.skill })).into_response()
}

// ── background processes ──────────────────────────────────────────────────

/// GET /api/bg — list registered background processes (global).
pub async fn list_bg(State(_state): State<Arc<AppState>>) -> Response {
    let procs: Vec<serde_json::Value> = opencoder_session::tools::bg::list()
        .into_iter()
        .map(|info| {
            json!({
                "pid": info.pid,
                "output_path": info.output_path.to_string_lossy(),
            })
        })
        .collect();
    Json(json!({ "processes": procs })).into_response()
}

/// POST /api/bg/stop — kill all background processes.
pub async fn stop_bg(State(_state): State<Arc<AppState>>) -> Response {
    let killed = opencoder_session::tools::bg::kill_all();
    Json(json!({ "ok": true, "killed": killed })).into_response()
}

// ── helpers ───────────────────────────────────────────────────────────────

fn load_config(state: &AppState) -> Result<Config, Box<Response>> {
    Config::load(&state.workdir).map_err(|e| Box::new(error_500(format!("config: {e:#}"))))
}

fn build_client(
    state: &AppState,
    config: &Config,
) -> Result<Arc<dyn ChatStream>, Box<Response>> {
    if let Some(c) = state.client_override.clone() {
        return Ok(c);
    }
    let ep = match config.resolve_endpoint() {
        Ok(v) => v,
        Err(e) => return Err(Box::new(error_500(format!("api_key: {e:#}")))),
    };
    match ChatClient::new_with_read_timeout(
        &ep.base_url,
        &ep.api_key,
        &ep.headers,
        config.stream_idle_timeout(),
        config.network.proxy.as_deref(),
    ) {
        Ok(c) => Ok(Arc::new(c) as Arc<dyn ChatStream>),
        Err(e) => Err(Box::new(error_500(format!("client: {e:#}")))),
    }
}

fn error_404(msg: &str) -> Response {
    (
        axum::http::StatusCode::NOT_FOUND,
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
