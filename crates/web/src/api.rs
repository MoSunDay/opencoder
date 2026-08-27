//! HTTP handlers. The prompt endpoint admits durably and returns immediately;
//! streaming happens via the SSE `/events` endpoint. Agent/model switches and
//! interrupt mutate the live session handle.

// Events endpoints live in `api_events.rs` (file-size budget); re-exported so
// router wiring and tests keep using `api::get_events` / `api::get_event_seq`.
pub use crate::api_events::{get_event_seq, get_events, EventsQuery};

use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use tracing::warn;

use opencoder_core::Config;
use opencoder_llm::{ChatClient, ChatStream};
use opencoder_store::{Delivery, EventKind, SessionFilter, SessionMeta, SessionPatch};

use crate::handle::{admit_and_drain_guarded, AdmissionError, SseEvt};
use crate::AppState;

#[derive(Deserialize)]
pub struct CreateBody {
    agent: Option<String>,
    model: Option<String>,
}

pub async fn create_session(
    State(state): State<Arc<AppState>>,
    body: Option<Json<CreateBody>>,
) -> impl IntoResponse {
    let id = opencoder_session::runner::new_id();
    let now = opencoder_core::message::now_ms();
    let meta = SessionMeta {
        id: id.clone(),
        title: None,
        agent: body
            .as_ref()
            .and_then(|b| b.agent.clone())
            .or_else(|| Some("act".into())),
        model: body.as_ref().and_then(|b| b.model.clone()),

        autopilot_mode: None,
        // Stamp the server workdir so `?workdir=` filters can match (legacy
        // NULL-hash rows never match).
        workdir_hash: Some(opencoder_core::workdir_hash(&state.workdir)),
        created_at: now,
        updated_at: now,
        summary: None,
        summary_seq: None,
        summary_images: vec![],
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
        requirement: None,
        plan_snapshot: None,
        plan_input_count: 0,
    };
    if let Err(e) = state.store.create_session(&meta).await {
        return error_500(format!("create_session: {e:#}"));
    }
    Json(json!({ "id": id })).into_response()
}

#[derive(Deserialize, Default)]
pub struct ListQuery {
    pub limit: Option<u32>,
    pub cursor: Option<String>,
    pub search: Option<String>,
    /// Optional workdir filter: only sessions created under this exact
    /// (canonicalized) directory; NULL-hash legacy rows never match.
    pub workdir: Option<String>,
}

pub async fn list_sessions(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ListQuery>,
) -> Result<Response, Response> {
    let filter = SessionFilter {
        limit: q.limit.unwrap_or(50).clamp(1, 500),
        cursor: q.cursor,
        workdir_hash: q
            .workdir
            .as_deref()
            .map(|w| opencoder_core::workdir_hash(std::path::Path::new(w))),
        search: q.search,
        include_subagents: false,
    };
    let items = state
        .store
        .list_sessions(&filter)
        .await
        .map_err(|e| error_500(format!("list_sessions: {e:#}")))?;
    Ok(Json(json!({ "sessions": items })).into_response())
}

pub async fn get_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    messages_response(&state, &id).await
}

/// DELETE /api/sessions/:id — delete a session. Cascades to its messages,
/// inputs, events, and subagent tasks via `ON DELETE CASCADE`. Returns 404 when
/// no session exists for `id`, 200 on success (idempotent for absent ids).
pub async fn delete_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    // Distinguish "did not exist" from "deleted" so callers get a real 404.
    // Distinguish "did not exist" (404) from a genuine DB error (500); the old
    // `unwrap_or(None)` masked storage failures as "session not found".
    let existed = match state.store.get_session(&id).await {
        Ok(m) => m,
        Err(e) => return error_500(format!("get_session: {e:#}")),
    };
    if existed.is_none() {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "error": format!("session not found: {id}") })),
        )
            .into_response();
    }
    match state.store.delete_session(&id).await {
        Ok(()) => {
            // Evict the live handle and cancel any running drain so a deleted
            // session stops calling the LLM (otherwise the drain outlives the
            // DELETE and keeps making LLM requests on a gone session).
            if let Some(h) = state.handles.lock().await.remove(&id) {
                h.cancel.lock().await.cancel();
                opencoder_session::fire_child_cancels(&h.child_cancels);
            }
            opencoder_session::mcp::cleanup(&id).await; // MCP connection cleanup
            Json(json!({ "ok": true, "id": id })).into_response()
        }
        Err(e) => error_500(format!("delete_session: {e:#}")),
    }
}

pub async fn get_messages(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    messages_response(&state, &id).await
}

async fn messages_response(state: &AppState, id: &str) -> Result<Response, Response> {
    let meta = match state.store.get_session(id).await {
        Ok(m) => m,
        Err(e) => return Err(error_500(format!("get_session: {e:#}"))),
    };
    let messages = state
        .store
        .load_messages(id)
        .await
        .map_err(|e| error_500(format!("load_messages: {e:#}")))?;
    Ok(Json(json!({ "id": id, "meta": meta, "messages": messages })).into_response())
}

#[derive(Deserialize)]
pub struct PromptBody {
    pub prompt: String,
    /// Optional image attachments as data URIs (`data:image/<fmt>;base64,...`)
    /// or `http(s)://` URLs. Forwarded to `SessionInput.images` so vision
    /// models receive them. Empty/absent for plain-text prompts.
    #[serde(default)]
    pub images: Vec<String>,
    pub delivery: Option<String>,
    pub agent: Option<String>,
    pub model: Option<String>,
    /// Optional skill name. Persisted to session meta and live-applied to
    /// the drain's skill handle. One-shot: the run that consumes it clears
    /// the skill at run end (a crash mid-run keeps it until the resumed run
    /// ends).
    pub skill: Option<String>,
}

#[derive(Deserialize)]
pub struct SubagentSteerBody {
    pub prompt: String,
    #[serde(default)]
    pub images: Vec<String>,
}

/// Admit a prompt durably, ensure a drain is running, return immediately with
/// the admitted seq. The client then streams `/events` for the live result.
pub async fn post_prompt(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(mut body): Json<PromptBody>,
) -> Response {
    if let Some(resp) = reject_node_session(&state, &id).await {
        return resp;
    }
    // Only an explicit `agent` field is an admission-time mode change: it
    // rewrites the session config, so it must be refused while a drain runs.
    // Textual mode commands (/plan, /act, /act_clear_context) are admitted
    // like any prompt — the runner applies them at the next idle/turn
    // boundary, which structurally has no turn in flight.
    let agent_override = body.agent.is_some();
    // Fast-path an already-running agent-field request before config/client
    // work so the stable busy contract wins even if configuration changed
    // mid-run. The guarded admission below repeats this check under the
    // lifecycle lock to close a drain-start race after this read.
    if agent_override
        && state
            .handles
            .lock()
            .await
            .get(&id)
            .is_some_and(|handle| handle.draining.load(Ordering::SeqCst))
    {
        return error_409("mode switch refused while drain running");
    }
    let mut config = match Config::load(&state.workdir) {
        Ok(c) => c,
        Err(e) => return error_500(format!("config: {e:#}")),
    };
    if let Some(resp) = crate::api_ops::apply_prompt_model(&mut config, body.model.take()) {
        return resp;
    }
    if let Some(a) = &body.agent {
        config.agent.default = a.clone();
    }
    // Use an injected client when present (tests), otherwise build a real
    // `ChatClient` from config + the resolved API key (production).
    let client: Arc<dyn ChatStream> = match state.client_override.clone() {
        Some(c) => c,
        None => {
            let ep = match config.resolve_endpoint() {
                Ok(v) => v,
                Err(e) => return error_500(format!("api_key: {e:#}")),
            };
            match ChatClient::new_with_read_timeout(
                &ep.base_url,
                &ep.api_key,
                &ep.headers,
                config.stream_idle_timeout(),
                config.network.proxy.as_deref(),
            ) {
                Ok(c) => Arc::new(c),
                Err(e) => return error_500(format!("client: {e:#}")),
            }
        }
    };
    // A present-but-unparseable delivery (e.g. a "stear" typo) must be a 400
    // rather than silently degrade to `Steer` — that fallback would interrupt
    // the running turn. A missing field keeps the `Steer` default. `parse`
    // trims and lowercases, so `" queue "` is accepted as Queue.
    let delivery = match body.delivery.as_deref() {
        None => Delivery::Steer,
        Some(s) => match Delivery::parse(s) {
            Some(d) => d,
            None => {
                return error_400(format!(
                    "invalid delivery {s:?}: expected \"steer\" or \"queue\""
                ))
            }
        },
    };
    if let Err(e) = ensure_session_row(&state, &id, &body.prompt, &config).await {
        return error_500(e);
    }
    match admit_and_drain_guarded(
        state.handles.clone(),
        state.store.clone(),
        &id,
        body.prompt,
        std::mem::take(&mut body.images),
        delivery,
        client,
        state.workdir.clone(),
        config,
        body.skill,
        agent_override,
    )
    .await
    {
        Ok(seq) => Json(json!({ "admitted_seq": seq, "ok": true })).into_response(),
        Err(AdmissionError::BusyModeSwitch) => error_409("mode switch refused while drain running"),
        Err(AdmissionError::Other(e)) => error_500(format!("admit: {e:#}")),
    }
}

async fn ensure_session_row(
    state: &AppState,
    id: &str,
    prompt: &str,
    config: &Config,
) -> Result<(), String> {
    match state.store.get_session(id).await {
        Ok(Some(_)) => return Ok(()),
        Ok(None) => {}
        Err(e) => return Err(format!("get_session: {e:#}")),
    }
    let now = opencoder_core::message::now_ms();
    state
        .store
        .create_session(&SessionMeta {
            id: id.to_string(),
            title: Some(prompt.chars().take(80).collect()),
            agent: Some(config.agent.default.clone()),
            model: Some(config.model.clone()),

            autopilot_mode: None,
            // Same stamping as create_session: `?workdir=` filters match.
            workdir_hash: Some(opencoder_core::workdir_hash(&state.workdir)),
            created_at: now,
            updated_at: now,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
            plan_snapshot: None,
            plan_input_count: 0,
        })
        .await
        .map_err(|e| format!("create_session: {e:#}"))
}

#[derive(Deserialize)]
pub struct SwitchBody {
    pub value: String,
}

pub async fn post_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<SwitchBody>,
) -> Response {
    if let Some(resp) = reject_node_session(&state, &id).await {
        return resp;
    }
    // get-or-create the handle so the override is never dropped, even right
    // after create_session when no drain has started yet.
    let (handle, _lifecycle) =
        crate::handle_lifecycle::lock_session_lifecycle(&state.handles, &id).await;
    if handle.draining.load(Ordering::SeqCst) {
        return error_409("agent switch refused while drain running");
    }
    let plan_input_count = (body.value == "plan").then_some(0);
    if let Err(e) = state
        .store
        .update_session(
            &id,
            &SessionPatch {
                agent: Some(body.value.clone()),
                plan_input_count,
                updated_at: Some(opencoder_core::message::now_ms()),
                ..Default::default()
            },
        )
        .await
    {
        return error_500(format!("update_session: {e:#}"));
    }
    handle.overrides.lock().await.agent = Some(body.value.clone());
    Json(json!({ "ok": true, "agent": body.value })).into_response()
}

/// Request body for `POST /sessions/:id/model`.
///
/// `persist_default = true` additionally writes the model as the global
/// default in `opencoder.json` via `Config::save` (defaults to false, i.e.
/// session-only — matching the TUI's default).
#[derive(Deserialize)]
pub struct ModelBody {
    pub value: String,
    #[serde(default)]
    pub persist_default: bool,
}

pub async fn post_model(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<ModelBody>,
) -> Response {
    if let Some(resp) = reject_node_session(&state, &id).await {
        return resp;
    }
    // get-or-create the handle so the override is never dropped, even right
    // after create_session when no drain has started yet.
    let (handle, _lifecycle) =
        crate::handle_lifecycle::lock_session_lifecycle(&state.handles, &id).await;
    if handle.draining.load(Ordering::SeqCst) {
        return error_409("model switch refused while drain running");
    }
    // Keep the old value only for a global-config save failure rollback.
    let old_model = match state.store.get_session(&id).await {
        Ok(m) => m,
        Err(e) => {
            warn!(error = %e, session_id = %id, "post_model: get_session for rollback failed");
            None
        }
    };
    if let Err(e) = state
        .store
        .update_session(
            &id,
            &SessionPatch {
                model: Some(body.value.clone()),
                updated_at: Some(opencoder_core::message::now_ms()),
                ..Default::default()
            },
        )
        .await
    {
        return error_500(format!("update_session: {e:#}"));
    }
    // A refused request never mutates the global default. Save failure rolls
    // back the session meta while the lifecycle lock still excludes a drain.
    if body.persist_default {
        let patch = serde_json::json!({ "model": &body.value });
        if let Err(e) = Config::save(&state.workdir, &patch) {
            let rollback = SessionPatch::rollback_model(old_model.as_ref());
            let _ = state.store.update_session(&id, &rollback).await;
            return error_500(format!("persist_default failed: {e:#}"));
        }
    }
    handle.overrides.lock().await.model = Some(body.value.clone());
    Json(json!({ "ok": true, "model": body.value })).into_response()
}

pub async fn post_interrupt(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    // Node sessions are interrupted through their task's cancel route, not
    // this drain-oriented endpoint.
    if let Some(resp) = reject_node_session(&state, &id).await {
        return resp;
    }
    let handle = state.handles.lock().await.get(&id).cloned();
    match &handle {
        // Only an actively draining session can be interrupted; a stale/idle
        // handle has no live drain task to cancel.
        Some(h) if h.draining.load(Ordering::SeqCst) => {
            h.cancel.lock().await.cancel();
            opencoder_session::fire_child_cancels(&h.child_cancels);
            Json(json!({ "ok": true })).into_response()
        }
        Some(_) => Json(json!({ "ok": false, "error": "no active drain running" })).into_response(),
        None => Json(json!({ "ok": false, "error": "no active session handle" })).into_response(),
    }
}

/// Steer a running subagent: admit a steer input to the child session and fire
/// the child's turn-cancel token so the steer is absorbed at the next turn
/// boundary. Requires the subagent task to exist and be `Running`.
pub async fn post_subagent_steer(
    State(state): State<Arc<AppState>>,
    Path((id, task_id)): Path<(String, String)>,
    Json(body): Json<SubagentSteerBody>,
) -> Response {
    use opencoder_store::{Delivery, SessionInput, SubagentStatus};

    if let Some(resp) = reject_node_session(&state, &id).await {
        return resp;
    }
    if opencoder_session::control_cmd::is_mode_control(&body.prompt) {
        return error_409("mode switch refused while subagent running");
    }

    // Guard: task must exist and be running.
    let task = match state.store.get_subagent_task(&task_id).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            return (
                axum::http::StatusCode::NOT_FOUND,
                Json(json!({ "ok": false, "error": "subagent task not found" })),
            )
                .into_response();
        }
        Err(e) => return error_500(format!("get_subagent_task: {e:#}")),
    };
    // The URL path `:id` is the parent session; ensure the fetched task
    // actually belongs to it so a task_id from another session can't be
    // steered through this route.
    if task.parent_session_id != id {
        return error_404("task not found in this session");
    }
    if task.status != SubagentStatus::Running {
        return (
            axum::http::StatusCode::CONFLICT,
            Json(json!({ "ok": false, "error": "subagent is not running" })),
        )
            .into_response();
    }

    // Reserve admission against the live child runner before the async store
    // write. A stale `Running` DB row alone is not sufficient: once the gate
    // closes, no runner exists that could consume another steer.
    let handle = state.handles.lock().await.get(&id).cloned();
    let gate = handle.as_ref().and_then(|h| {
        h.child_steer_gates
            .lock()
            .ok()
            .and_then(|map| map.get(&task_id).cloned())
    });
    let Some(reservation) = gate.and_then(|gate| gate.reserve()) else {
        return (
            axum::http::StatusCode::CONFLICT,
            Json(json!({
                "ok": false,
                "error": "subagent is no longer accepting steers"
            })),
        )
            .into_response();
    };

    // Admit the steer to the child session (no drain — the child's run_loop
    // is already running inside the parent's tool execution).
    let input = SessionInput {
        seq: None,
        id: uuid::Uuid::new_v4().to_string(),
        session_id: task.child_session_id.clone(),
        delivery: Delivery::Steer,
        prompt: body.prompt,
        images: body.images,
        display_text: None,
        admitted_seq: 0,
        promoted_seq: None,
    };
    let seq = match state.store.admit_input(&input).await {
        Ok(s) => s,
        Err(e) => return error_500(format!("admit: {e:#}")),
    };

    if !reservation.commit() {
        // Best-effort cleanup (Bug 9): if already promoted, delete is a no-op.
        if let Err(e) = state.store.delete_input(seq).await {
            return error_500(format!("subagent steer rollback: {e:#}"));
        }
        return (
            axum::http::StatusCode::CONFLICT,
            Json(json!({
                "ok": false,
                "error": "subagent is no longer accepting steers"
            })),
        )
            .into_response();
    }

    // Fire the child's turn-cancel token to interrupt the current turn,
    // forcing the steer to be absorbed at the next turn boundary.
    if let Some(h) = handle {
        if let Ok(g) = h.child_turn_cancels.lock() {
            if let Some(token) = g.get(&task_id).cloned() {
                if let Ok(t) = token.lock() {
                    t.cancel();
                }
            }
        }
    }

    Json(json!({ "admitted_seq": seq, "ok": true })).into_response()
}

pub async fn health() -> impl IntoResponse {
    Json(json!({
        "ok": true,
        "version": opencoder_core::version::VERSION_LONG,
        "commit": opencoder_core::version::GIT_COMMIT,
    }))
}

/// Shared mutation gate for session-scoped endpoints: synthetic node sessions
/// (`task_type == "node"`) are executed by worker nodes, never by this
/// server's drain loop — any prompt/switch/interrupt/fork against them would
/// desync the node protocol. Returns `Some(409)` to short-circuit the handler.
pub(crate) async fn reject_node_session(state: &AppState, session_id: &str) -> Option<Response> {
    match state.store.get_session(session_id).await {
        Ok(Some(meta)) if meta.task_type.as_deref() == Some(opencoder_store::TASK_TYPE_NODE) => {
            Some(
                (
                    axum::http::StatusCode::CONFLICT,
                    Json(json!({
                        "ok": false,
                        "error": "synthetic node session; use /api/nodes/…"
                    })),
                )
                    .into_response(),
            )
        }
        Ok(_) => None,
        Err(e) => Some(error_500(format!("get_session: {e:#}"))),
    }
}

pub(crate) fn error_400(msg: String) -> Response {
    (
        axum::http::StatusCode::BAD_REQUEST,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}
pub(crate) fn error_409(msg: &str) -> Response {
    (
        axum::http::StatusCode::CONFLICT,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}
pub(crate) fn error_404(msg: &str) -> Response {
    (
        axum::http::StatusCode::NOT_FOUND,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

pub(crate) fn error_500(msg: String) -> Response {
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

/// 502 Bad Gateway: the node (our upstream) answered the relay with a
/// failure. Used by the P3 message relay; the payload is never persisted.
pub(crate) fn error_502(msg: String) -> Response {
    (
        axum::http::StatusCode::BAD_GATEWAY,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

pub(crate) fn event_kind_str(k: EventKind) -> &'static str {
    match k {
        EventKind::PromptAdmitted => "prompt_admitted",
        EventKind::PromptPromoted => "prompt_promoted",
        EventKind::TextDelta => "text_delta",
        EventKind::ToolStart => "tool_start",
        EventKind::ToolEnd => "tool_end",
        EventKind::AgentSwitched => "agent_switched",
        EventKind::ModelSwitched => "model_switched",
        EventKind::Compaction => "compaction",
        EventKind::Step => "status",
        EventKind::Interrupted => "interrupted",
        EventKind::Done => "done",
        EventKind::Error => "error",
    }
}

/// Map a `BroadcastStream` item (Ok event / Err lag) to an `SseEvt`. A lag used
/// to be silently dropped (`r.ok()`), which could swallow a terminal
/// `done`/`error` event and freeze the UI (busy never resets). Now it is
/// surfaced as a synthetic `error` event so the client knows it must re-sync.
/// Pure so the lag handling is directly unit-testable.
pub fn map_broadcast_result(
    r: Result<SseEvt, tokio_stream::wrappers::errors::BroadcastStreamRecvError>,
) -> Option<SseEvt> {
    match r {
        Ok(evt) => Some(evt),
        Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n)) => Some(SseEvt {
            kind: "error".into(),
            data: json!({ "error": format!("event lag: {n} events dropped") }),
            ts: opencoder_core::message::now_ms(),
            seq: None,
        }),
    }
}
