//! HTTP handlers. The prompt endpoint admits durably and returns immediately;
//! streaming happens via the SSE `/events` endpoint. Agent/model switches and
//! interrupt mutate the live session handle.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::stream::StreamExt;
use serde::Deserialize;
use serde_json::json;
use tracing::warn;

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
}

pub async fn list_sessions(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ListQuery>,
) -> Result<Response, Response> {
    let filter = SessionFilter {
        limit: q.limit.unwrap_or(50).clamp(1, 500),
        cursor: q.cursor,
        workdir_hash: None,
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
    /// Optional skill name. Persisted to session meta and live-applied to a
    /// running drain (resume restores it automatically).
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
    // Persist skill if provided (resume will restore it on the next drain).
    if let Some(skill) = &body.skill {
        if let Err(e) = state
            .store
            .update_session(
                &id,
                &SessionPatch {
                    skill: Some(skill.clone()),
                    updated_at: Some(opencoder_core::message::now_ms()),
                    ..Default::default()
                },
            )
            .await
        {
            return error_500(format!("persist skill: {e:#}"));
        }
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
            workdir_hash: None,
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
    // get-or-create the handle so the override is never dropped, even right
    // after create_session when no drain has started yet.
    let handle = {
        let mut map = state.handles.lock().await;
        map.entry(id.clone())
            .or_insert_with(SessionHandle::new)
            .clone()
    };
    // RUNNING-GATE: an actively draining session must not be mode-switched —
    // the live turn keeps its current agent, so applying the switch now would
    // leave chat.agent / persisted meta diverging from the executing mode.
    // Refuse BEFORE any store-meta or override mutation (atomicity). Mirrors
    // `post_interrupt`'s draining gate.
    if handle.draining.load(Ordering::SeqCst) {
        return error_409("agent switch refused while drain running");
    }
    // P1-5: Capture old meta for TOCTOU rollback. A drain may start between
    // the draining check above and the update_session write below.
    let old_agent = match state.store.get_session(&id).await {
        Ok(m) => m,
        Err(e) => {
            warn!(error = %e, session_id = %id, "post_agent: get_session for rollback failed");
            None
        }
    };
    if let Err(e) = state
        .store
        .update_session(
            &id,
            &SessionPatch {
                agent: Some(body.value.clone()),
                updated_at: Some(opencoder_core::message::now_ms()),
                ..Default::default()
            },
        )
        .await
    {
        return error_500(format!("update_session: {e:#}"));
    }
    // P1-5: Re-check draining AFTER the write. If a drain started between
    // the first check and this write (TOCTOU), rollback the meta change.
    // `rollback_agent` restores the captured value — or CLEARS the column
    // when it was NULL / the capture read failed: a plain `agent: None`
    // patch is a no-op and would leave the refused switch persisted.
    if handle.draining.load(Ordering::SeqCst) {
        let patch = SessionPatch::rollback_agent(old_agent.as_ref());
        let _ = state.store.update_session(&id, &patch).await;
        return error_409("agent switch refused: drain started during write");
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
    // get-or-create the handle so the override is never dropped, even right
    // after create_session when no drain has started yet.
    let handle = {
        let mut map = state.handles.lock().await;
        map.entry(id.clone())
            .or_insert_with(SessionHandle::new)
            .clone()
    };
    // RUNNING-GATE: a model override has the identical deferred-next-drain
    // semantics as the agent switch — a mid-turn model switch would diverge
    // from the model actually executing the live turn. Refuse BEFORE any
    // config / store-meta / override mutation (atomicity).
    if handle.draining.load(Ordering::SeqCst) {
        return error_409("model switch refused while drain running");
    }
    // P1-5: Capture old meta for TOCTOU rollback.
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
    // P1-5: Re-check draining AFTER the write (TOCTOU). Rollback meta if a
    // drain started during the write. `rollback_model` clears the column when
    // the old value was NULL / unreadable (a `model: None` patch is a no-op).
    if handle.draining.load(Ordering::SeqCst) {
        let patch = SessionPatch::rollback_model(old_model.as_ref());
        let _ = state.store.update_session(&id, &patch).await;
        return error_409("model switch refused: drain started during write");
    }
    // Persist global config only after all drain checks pass (Bug 6): a refused
    // request never mutates the global default. Save failure rolls back meta
    // (same NULL-clearing semantics as the drain rollback above).
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
) -> impl IntoResponse {
    let handle = state.handles.lock().await.get(&id).cloned();
    match &handle {
        // Only an actively draining session can be interrupted; a stale/idle
        // handle has no live drain task to cancel.
        Some(h) if h.draining.load(Ordering::SeqCst) => {
            h.cancel.lock().await.cancel();
            opencoder_session::fire_child_cancels(&h.child_cancels);
            Json(json!({ "ok": true }))
        }
        Some(_) => Json(json!({ "ok": false, "error": "no active drain running" })),
        None => Json(json!({ "ok": false, "error": "no active session handle" })),
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
) -> Response {
    let after = q.after.unwrap_or(0);

    // Reject non-existent sessions: otherwise a get-or-created handle would
    // subscribe to a broadcast that never fires, hanging the SSE stream forever.
    match state.store.get_session(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return error_404("session not found"),
        Err(e) => return error_500(format!("get_session: {e:#}")),
    }

    // Subscribe FIRST, then query persisted events. This closes the race where
    // an event broadcast between query and subscribe is lost (not yet
    // persisted at query time, not received via broadcast). With subscribe-first
    // every post-subscribe broadcast is captured by the live stream; any overlap
    // with the replay window is deduplicated below.
    let (rx, created) = {
        let mut map = state.handles.lock().await;
        let created = !map.contains_key(&id);
        let handle = map.entry(id.clone()).or_insert_with(SessionHandle::new);
        // Track this subscriber so the handle this request may have created is
        // evicted (see `release_events_subscriber`) once everyone disconnects.
        handle.subscribers.fetch_add(1, Ordering::SeqCst);
        (handle.tx.subscribe(), created)
    };

    // P0-1: Capture the persisted-seq baseline BEFORE querying `events_after`.
    // `last_event_seq` returns the current max persisted seq; reading it AFTER
    // `events_after` (as the original code did) guarantees `baseline >=
    // max(seq)`, so the `seq > baseline` filter below is ALWAYS false and `seen`
    // (the tier-(2) content-dedup set) stays permanently empty. Snapshotting it
    // here — immediately after subscribing — means any event persisted in the
    // window between this snapshot and the `events_after` query (seq > baseline)
    // is a genuine subscribe/query overlap-window event that must seed `seen`.
    let baseline = state.store.last_event_seq(&id).await.unwrap_or(-1);

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

    // Dedup live broadcast events against the replayed (persisted) window:
    // two-tier decision (exact seq, then content fingerprint) + overlap-window
    // seeding and the first-forwarded-`done` TTL live in `sse_dedup`.
    let max_replay_seq: i64 = persisted.iter().filter_map(|e| e.seq).max().unwrap_or(-1);
    let seen = crate::sse_dedup::seed_seen(&persisted, baseline);

    let replay = futures::stream::iter(persisted);
    let live = tokio_stream::wrappers::BroadcastStream::new(rx)
        .filter_map(|r| async move { map_broadcast_result(r) })
        .filter_map({
            let seen = Arc::clone(&seen);
            move |evt| {
                let seen = Arc::clone(&seen);
                async move {
                    crate::sse_dedup::forward_live(&evt, &seen, max_replay_seq).then_some(evt)
                }
            }
        });
    let merged = replay.chain(live).map(|evt| {
        let data = serde_json::to_string(&evt.data).unwrap_or_else(|_| "{}".into());
        Ok::<_, std::convert::Infallible>(Event::default().event(evt.kind).data(data))
    });

    // Wrap in a drop guard so that when the client disconnects (or the stream
    // ends) this request's subscriber slot is released and, if it created the
    // handle and nothing remains, the handle is evicted — preventing unbounded
    // handle-map growth on a long-running server.
    let guarded = crate::handle::DropGuardStream::new(merged, move || {
        crate::handle::release_events_subscriber(state.handles.clone(), id, created)
    });

    Sse::new(guarded)
        .keep_alive(KeepAlive::default())
        .into_response()
}

pub async fn health() -> impl IntoResponse {
    Json(json!({
        "ok": true,
        "version": opencoder_core::version::VERSION_LONG,
        "commit": opencoder_core::version::GIT_COMMIT,
    }))
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

fn error_400(msg: String) -> Response {
    (
        axum::http::StatusCode::BAD_REQUEST,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}
fn error_409(msg: &str) -> Response {
    (
        axum::http::StatusCode::CONFLICT,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
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
