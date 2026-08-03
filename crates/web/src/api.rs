//! HTTP handlers. The prompt endpoint admits durably and returns immediately;
//! streaming happens via the SSE `/events` endpoint. Agent/model switches and
//! interrupt mutate the live session handle.

use std::collections::HashSet;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::stream::StreamExt;
use serde::Deserialize;
use serde_json::json;

use opencoder_core::Config;
use opencoder_llm::{ChatClient, ChatStream};
use opencoder_store::{Delivery, EventKind, SessionFilter, SessionMeta, SessionPatch};

use crate::handle::{admit_and_drain, SessionHandle, SseEvt};
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
        workdir_hash: None,
        created_at: now,
        updated_at: now,
        summary: None,
        summary_seq: None,
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
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
}

pub async fn list_sessions(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let filter = SessionFilter {
        limit: q.limit.unwrap_or(50).clamp(1, 500),
        cursor: q.cursor,
        workdir_hash: None,
        search: q.search,
        include_subagents: false,
    };
    let items = state.store.list_sessions(&filter).await.unwrap_or_default();
    Json(json!({ "sessions": items })).into_response()
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
    let existed = state.store.get_session(&id).await.unwrap_or(None);
    if existed.is_none() {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "error": format!("session not found: {id}") })),
        )
            .into_response();
    }
    match state.store.delete_session(&id).await {
        Ok(()) => Json(json!({ "ok": true, "id": id })).into_response(),
        Err(e) => error_500(format!("delete_session: {e:#}")),
    }
}

pub async fn get_messages(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    messages_response(&state, &id).await
}

async fn messages_response(state: &AppState, id: &str) -> Response {
    let meta = state.store.get_session(id).await.ok().flatten();
    let messages = state.store.load_messages(id).await.unwrap_or_default();
    Json(json!({ "id": id, "meta": meta, "messages": messages })).into_response()
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
    let mut config = match Config::load(&state.workdir) {
        Ok(c) => c,
        Err(e) => return error_500(format!("config: {e:#}")),
    };
    if let Some(m) = body.model {
        config.model = m;
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
    let delivery = body
        .delivery
        .as_deref()
        .and_then(Delivery::parse)
        .unwrap_or(Delivery::Steer);
    if let Err(e) = ensure_session_row(&state, &id, &body.prompt, &config).await {
        return error_500(e);
    }
    match admit_and_drain(
        state.handles.clone(),
        state.store.clone(),
        &id,
        body.prompt,
        std::mem::take(&mut body.images),
        delivery,
        client,
        state.workdir.clone(),
        config,
    )
    .await
    {
        Ok(seq) => Json(json!({ "admitted_seq": seq, "ok": true })).into_response(),
        Err(e) => error_500(format!("admit: {e:#}")),
    }
}

async fn ensure_session_row(
    state: &AppState,
    id: &str,
    prompt: &str,
    config: &Config,
) -> Result<(), String> {
    if state.store.get_session(id).await.ok().flatten().is_some() {
        return Ok(());
    }
    let now = opencoder_core::message::now_ms();
    state
        .store
        .create_session(&SessionMeta {
            id: id.to_string(),
            title: Some(prompt.chars().take(80).collect()),
            agent: Some(config.agent.default.clone()),
            model: Some(config.model.clone()),
            workdir_hash: None,
            created_at: now,
            updated_at: now,
            summary: None,
            summary_seq: None,
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
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
) -> impl IntoResponse {
    let _ = state
        .store
        .update_session(
            &id,
            &SessionPatch {
                agent: Some(body.value.clone()),
                updated_at: Some(opencoder_core::message::now_ms()),
                ..Default::default()
            },
        )
        .await;
    if let Some(h) = state.handles.lock().await.get(&id).cloned() {
        h.overrides.lock().await.agent = Some(body.value.clone());
    }
    Json(json!({ "ok": true, "agent": body.value }))
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
    let _ = state
        .store
        .update_session(
            &id,
            &SessionPatch {
                model: Some(body.value.clone()),
                updated_at: Some(opencoder_core::message::now_ms()),
                ..Default::default()
            },
        )
        .await;
    if let Some(h) = state.handles.lock().await.get(&id).cloned() {
        h.overrides.lock().await.model = Some(body.value.clone());
    }
    if body.persist_default {
        let patch = serde_json::json!({ "model": &body.value });
        if let Err(e) = Config::save(&state.workdir, &patch) {
            return error_500(format!("persist_default failed: {e:#}"));
        }
    }
    Json(json!({ "ok": true, "model": body.value })).into_response()
}

pub async fn post_interrupt(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let handle = state.handles.lock().await.get(&id).cloned();
    if let Some(h) = &handle {
        h.cancel.lock().await.cancel();
    }
    if handle.is_some() {
        Json(json!({ "ok": true }))
    } else {
        Json(json!({ "ok": false, "error": "no active session handle" }))
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
    if task.status != SubagentStatus::Running {
        return (
            axum::http::StatusCode::CONFLICT,
            Json(json!({ "ok": false, "error": "subagent is not running" })),
        )
            .into_response();
    }

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

    // Fire the child's turn-cancel token to interrupt the current turn,
    // forcing the steer to be absorbed at the next turn boundary.
    if let Some(h) = state.handles.lock().await.get(&id).cloned() {
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

#[derive(Deserialize, Default)]
pub struct EventsQuery {
    pub after: Option<i64>,
}

/// SSE stream: replay persisted events `after` the cursor, then forward the
/// live broadcast. Slow clients skip lagged events (backpressure never blocks
/// the runner); a missing live handle still yields the replay window.
pub async fn get_events(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<EventsQuery>,
) -> impl IntoResponse {
    let after = q.after.unwrap_or(0);

    // Subscribe FIRST, then query persisted events. This closes the race where
    // an event broadcast between query and subscribe is lost (not yet
    // persisted at query time, not received via broadcast). With subscribe-first
    // every post-subscribe broadcast is captured by the live stream; any overlap
    // with the replay window is deduplicated below.
    let rx = {
        let mut map = state.handles.lock().await;
        let handle = map.entry(id.clone()).or_insert_with(SessionHandle::new);
        handle.tx.subscribe()
    };

    let persisted: Vec<SseEvt> = state
        .store
        .events_after(&id, after)
        .await
        .map(|records| {
            records
                .into_iter()
                .map(|r| SseEvt {
                    kind: r
                        .sse_kind
                        .clone()
                        .unwrap_or_else(|| event_kind_str(r.kind).to_string()),
                    data: r.payload,
                    ts: r.ts,
                    seq: r.seq,
                })
                .collect()
        })
        .unwrap_or_default();

    // Dedup: events broadcast between subscribe and query may appear in BOTH
    // the replay (persisted before query) and the live stream (received via
    // broadcast). Track fingerprints of replayed events and skip matching live
    // events. The set is bounded by the replay size and shrinks as matches are
    // consumed, so it self-clears once the overlap window passes.
    let seen: Arc<std::sync::Mutex<HashSet<(String, String)>>> = Arc::new(
        std::sync::Mutex::new(
            persisted
                .iter()
                .map(|e| (e.kind.clone(), e.data.to_string()))
                .collect(),
        ),
    );

    let replay = futures::stream::iter(persisted);
    let live = tokio_stream::wrappers::BroadcastStream::new(rx)
        .filter_map(|r| async move { r.ok() })
        .filter_map({
            let seen = Arc::clone(&seen);
            move |evt| {
                let seen = Arc::clone(&seen);
                async move {
                    let key = (evt.kind.clone(), evt.data.to_string());
                    let mut is_dup = false;
                    if let Ok(mut guard) = seen.lock() {
                        if guard.contains(&key) {
                            guard.remove(&key);
                            is_dup = true;
                        }
                    }
                    if is_dup {
                        None
                    } else {
                        Some(evt)
                    }
                }
            }
        });
    let merged = replay.chain(live).map(|evt| {
        let data = serde_json::to_string(&evt.data).unwrap_or_else(|_| "{}".into());
        Ok::<_, std::convert::Infallible>(Event::default().event(evt.kind).data(data))
    });

    Sse::new(merged).keep_alive(KeepAlive::default())
}

pub async fn health() -> impl IntoResponse {
    Json(json!({ "ok": true }))
}

/// Highest persisted event seq for a session (0 if none). A remote client uses
/// this to snapshot before posting a prompt so it only streams the events
/// produced by its own turn.
pub async fn get_event_seq(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let seq = state.store.last_event_seq(&id).await.unwrap_or(0);
    Json(json!({ "id": id, "seq": seq }))
}

fn error_500(msg: String) -> Response {
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

fn event_kind_str(k: EventKind) -> &'static str {
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
