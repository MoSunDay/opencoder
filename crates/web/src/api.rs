//! HTTP handlers. The prompt endpoint admits durably and returns immediately;
//! streaming happens via the SSE `/events` endpoint. Agent/model switches and
//! interrupt mutate the live session handle.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::stream::StreamExt;
use serde::Deserialize;
use serde_json::json;

use opencode_core::Config;
use opencode_llm::{ChatClient, ChatStream};
use opencode_store::{Delivery, EventKind, SessionFilter, SessionMeta, SessionPatch};

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
    let id = opencode_session::runner::new_id();
    let now = opencode_core::message::now_ms();
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
    };
    let _ = state.store.create_session(&meta).await;
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
    pub delivery: Option<String>,
    pub agent: Option<String>,
    pub model: Option<String>,
}

/// Admit a prompt durably, ensure a drain is running, return immediately with
/// the admitted seq. The client then streams `/events` for the live result.
pub async fn post_prompt(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<PromptBody>,
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
    let api_key = match config.api_key() {
        Ok(k) => k,
        Err(e) => return error_500(format!("api_key: {e:#}")),
    };
    let client: Arc<dyn ChatStream> = match ChatClient::new(&config.provider.base_url, &api_key) {
        Ok(c) => Arc::new(c),
        Err(e) => return error_500(format!("client: {e:#}")),
    };
    let delivery = body
        .delivery
        .as_deref()
        .and_then(Delivery::parse)
        .unwrap_or(Delivery::Steer);
    ensure_session_row(&state, &id, &body.prompt, &config).await;
    match admit_and_drain(
        state.handles.clone(),
        state.store.clone(),
        &id,
        body.prompt,
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

async fn ensure_session_row(state: &AppState, id: &str, prompt: &str, config: &Config) {
    if state.store.get_session(id).await.ok().flatten().is_some() {
        return;
    }
    let now = opencode_core::message::now_ms();
    let _ = state
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
        })
        .await;
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
                updated_at: Some(opencode_core::message::now_ms()),
                ..Default::default()
            },
        )
        .await;
    if let Some(h) = state.handles.lock().await.get(&id).cloned() {
        h.overrides.lock().await.agent = Some(body.value.clone());
    }
    Json(json!({ "ok": true, "agent": body.value }))
}

pub async fn post_model(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<SwitchBody>,
) -> impl IntoResponse {
    let _ = state
        .store
        .update_session(
            &id,
            &SessionPatch {
                model: Some(body.value.clone()),
                updated_at: Some(opencode_core::message::now_ms()),
                ..Default::default()
            },
        )
        .await;
    if let Some(h) = state.handles.lock().await.get(&id).cloned() {
        h.overrides.lock().await.model = Some(body.value.clone());
    }
    Json(json!({ "ok": true, "model": body.value }))
}

pub async fn post_interrupt(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if let Some(h) = state.handles.lock().await.get(&id).cloned() {
        h.cancel.lock().await.cancel();
    }
    Json(json!({ "ok": true }))
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
    let persisted: Vec<SseEvt> = state
        .store
        .events_after(&id, after)
        .await
        .map(|records| {
            records
                .into_iter()
                .map(|r| SseEvt {
                    kind: event_kind_str(r.kind).to_string(),
                    data: r.payload,
                    ts: r.ts,
                    seq: r.seq,
                })
                .collect()
        })
        .unwrap_or_default();

    let rx = {
        let mut map = state.handles.lock().await;
        let handle = map.entry(id.clone()).or_insert_with(SessionHandle::new);
        handle.tx.subscribe()
    };

    let replay = futures::stream::iter(persisted);
    let live =
        tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(|r| async move { r.ok() });
    let merged = replay.chain(live).map(|evt| {
        let data = serde_json::to_string(&evt.data).unwrap_or_else(|_| "{}".into());
        Ok::<_, std::convert::Infallible>(Event::default().event(evt.kind).data(data))
    });

    Sse::new(merged).keep_alive(KeepAlive::default())
}

pub async fn health() -> impl IntoResponse {
    Json(json!({ "ok": true }))
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
