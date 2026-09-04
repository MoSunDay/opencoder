//! `/api/brain` — REST surface over the project brain (capability library).
//!
//! The brain runtime (`opencoder_brain::Runtime`) already owns validation,
//! embedding and persistence; handlers here stay thin: parse → call → map the
//! error class onto an HTTP status. Mapping contract:
//!
//! * `domain::validate` rejections (field-level, e.g. "summary must not be
//!   empty") → **400**, message passed through verbatim;
//! * unknown id → **404**: update raises the typed
//!   `opencoder_brain::BrainNotFound` marker (matched via `downcast_ref`,
//!   its `Display` is the historical "brain capability not found: {id}"
//!   body); get/delete probe existence directly and render the same shape;
//! * upstream embed failures → **502**: the runtime carries every
//!   `ChatStream::embed` failure (HTTP error, cardinality mismatch, empty
//!   vector) as the typed `opencoder_brain::EmbeddingFailed` marker
//!   (`Runtime::embed_one`), so a `downcast_ref` on the error is the exact
//!   boundary between "bad payload" and "embedding backend down" — validate
//!   messages are plain anyhow strings that never construct that type;
//! * anything else (store I/O, post-write invariant violations) → 500.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;

use anyhow::{bail, Result};

use opencoder_brain::CapabilityInput;
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent};
use opencoder_store::{BrainCapabilityDetail, Store};

use crate::AppState;

/// Default neighbour count for `POST /api/brain/search` when `k` is omitted.
pub const DEFAULT_SEARCH_K: u32 = 10;
/// Hard ceiling for the `k` parameter (keeps one request from scanning the
/// whole vector table).
pub const MAX_SEARCH_K: u32 = 50;

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

fn error_500(msg: String) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

fn error_502(msg: String) -> Response {
    (
        StatusCode::BAD_GATEWAY,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

/// Map a post-validate runtime failure: embed outage → 502, else 500.
/// The split is the typed [`opencoder_brain::EmbeddingFailed`] marker the
/// runtime raises for every upstream `ChatStream::embed` failure — see the
/// module docs for the full status contract.
fn map_brain_error(op: &str, err: anyhow::Error) -> Response {
    if err
        .downcast_ref::<opencoder_brain::EmbeddingFailed>()
        .is_some()
    {
        error_502(format!("{err:#}"))
    } else {
        error_500(format!("{op}: {err:#}"))
    }
}

/// Serialize one capability read-model FLAT: the record under `capability`
/// (id directly indexable) with `eng_inputs` beside it — list responses keep
/// the nested `Detail` shape, single-resource responses flatten it so callers
/// never need `capability.capability.id`.
fn detail_json(detail: BrainCapabilityDetail) -> serde_json::Value {
    json!({
        "ok": true,
        "capability": detail.capability,
        "eng_inputs": detail.eng_inputs,
    })
}

/// GET /api/brain/capabilities — every capability (newest first) with its
/// ordered exemplar inputs.
pub async fn list_capabilities(State(state): State<Arc<AppState>>) -> Response {
    match state.brain.list_capabilities().await {
        Ok(caps) => Json(json!({ "ok": true, "capabilities": caps })).into_response(),
        Err(e) => error_500(format!("list brain capabilities: {e:#}")),
    }
}

/// POST /api/brain/capabilities — validate then upsert (embed + persist).
/// The id is minted by the runtime and echoed back inside `capability`.
pub async fn create_capability(
    State(state): State<Arc<AppState>>,
    Json(input): Json<CapabilityInput>,
) -> Response {
    // Validate locally first so payload errors are a clean 400 even though
    // the runtime would reject them identically (but interleaved with its
    // own error classes).
    if let Err(e) = opencoder_brain::domain::validate(&input) {
        return error_400(e.to_string());
    }
    match state
        .brain
        .upsert_capability(&input, opencoder_core::message::now_ms())
        .await
    {
        Ok(detail) => (StatusCode::CREATED, Json(detail_json(detail))).into_response(),
        Err(e) => map_brain_error("upsert brain capability", e),
    }
}

/// GET /api/brain/capabilities/:id — one capability or 404.
pub async fn get_capability(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    match state.brain.get_capability(&id).await {
        Ok(Some(detail)) => Json(detail_json(detail)).into_response(),
        Ok(None) => error_404(&format!("brain capability not found: {id}")),
        Err(e) => error_500(format!("get brain capability: {e:#}")),
    }
}

/// PUT /api/brain/capabilities/:id — replace content + re-embed. Unknown id
/// → 404 (the runtime's typed `BrainNotFound` marker), payload → 400, embed
/// → 502.
pub async fn update_capability(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(input): Json<CapabilityInput>,
) -> Response {
    if let Err(e) = opencoder_brain::domain::validate(&input) {
        return error_400(e.to_string());
    }
    match state
        .brain
        .update_capability(&id, &input, opencoder_core::message::now_ms())
        .await
    {
        Ok(detail) => (StatusCode::OK, Json(detail_json(detail))).into_response(),
        Err(e) => {
            // Same typed split as `map_brain_error`, plus the local 404
            // branch: the runtime raises the typed `BrainNotFound` marker
            // for an unknown id, while its post-write invariant contexts
            // ("not found after insert/update") stay plain anyhow strings
            // and therefore fall through to 500.
            if e.downcast_ref::<opencoder_brain::EmbeddingFailed>()
                .is_some()
            {
                error_502(format!("{e:#}"))
            } else if e.downcast_ref::<opencoder_brain::BrainNotFound>().is_some() {
                error_404(&format!("{e:#}"))
            } else {
                error_500(format!("update brain capability: {e:#}"))
            }
        }
    }
}

/// DELETE /api/brain/capabilities/:id — 200 ok / 404 (existence is probed
/// first because the store delete is idempotent and would not tell).
pub async fn delete_capability(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    match state.brain.get_capability(&id).await {
        Ok(None) => error_404(&format!("brain capability not found: {id}")),
        Ok(Some(_)) => match state.brain.delete_capability(&id).await {
            Ok(()) => Json(json!({ "ok": true, "deleted": id })).into_response(),
            Err(e) => error_500(format!("delete brain capability: {e:#}")),
        },
        Err(e) => error_500(format!("get brain capability: {e:#}")),
    }
}

/// Body of `POST /api/brain/search`.
#[derive(Debug, Deserialize)]
pub struct SearchBody {
    pub query: String,
    /// Neighbour count; `None` → [`DEFAULT_SEARCH_K`], clamped to
    /// [`MAX_SEARCH_K`].
    pub k: Option<u32>,
}

/// POST /api/brain/search — nearest-neighbour search over capability
/// embeddings. Empty/blank query → 400, embed outage → 502.
pub async fn search(State(state): State<Arc<AppState>>, Json(body): Json<SearchBody>) -> Response {
    let query = body.query.trim();
    if query.is_empty() {
        return error_400("query must not be empty".to_string());
    }
    let k = body.k.unwrap_or(DEFAULT_SEARCH_K).clamp(1, MAX_SEARCH_K);
    match state.brain.search(query, k).await {
        Ok(hits) => Json(json!({ "ok": true, "hits": hits })).into_response(),
        Err(e) => map_brain_error("search brain", e),
    }
}

// ─── dynamic planning (decision trees over the capability library) ──────

/// Body of `POST /api/brain/plans`.
#[derive(Debug, Deserialize)]
pub struct PlanBody {
    pub situation: String,
    /// Vector candidates fed to the planner; `None` → [`DEFAULT_SEARCH_K`]
    /// clamped to [`MAX_SEARCH_K`] (same policy as search).
    pub top_k: Option<u32>,
    /// Planner chat model; `None` → the runtime's configured default
    /// (production: the config small model).
    pub model: Option<String>,
}

/// Body of `POST /api/brain/dispatch`.
#[derive(Debug, Deserialize)]
pub struct DispatchBody {
    pub situation: String,
    /// Route through one persisted plan; omit to auto-pick: the newest
    /// cached plan for this situation digest, planning one first when there
    /// is nothing to reuse.
    pub plan_id: Option<String>,
    /// Candidate count when a fresh plan must be minted.
    pub top_k: Option<u32>,
    /// Force re-planning even when a cached plan exists (default false).
    pub replan: Option<bool>,
    /// Planner chat model override (same default chain as `POST /plans`).
    pub model: Option<String>,
}

/// Map planner/dispatch failures: unknown plan id → 404 (typed
/// `PlanNotFound`), embed outage and planner-LLM faults → 502 (typed
/// `EmbeddingFailed` / `PlanGenerationFailed`), anything else (store I/O,
/// corrupt stored tree) → 500.
fn map_plan_error(op: &str, err: anyhow::Error) -> Response {
    if let Some(e) = err.downcast_ref::<opencoder_brain::PlanNotFound>() {
        error_404(&e.to_string())
    } else if err
        .downcast_ref::<opencoder_brain::EmbeddingFailed>()
        .is_some()
    {
        error_502(format!("{err:#}"))
    } else if err
        .downcast_ref::<opencoder_brain::PlanGenerationFailed>()
        .is_some()
    {
        error_502(format!("{op}: {err:#}"))
    } else {
        error_500(format!("{op}: {err:#}"))
    }
}

/// Resolve the planner chat model from a request override, falling back to
/// the runtime's configured default.
fn planner_model(state: &AppState, model: &Option<String>) -> String {
    model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| state.brain.chat_model().to_string())
}

/// POST /api/brain/plans — plan a decision tree for one situation: embed →
/// vector-retrieve candidates → framework-prompt LLM call → validated tree
/// (branch topics pre-embedded) → persisted. Empty situation → 400; empty
/// library / LLM faults → 502; embed outage → 502.
pub async fn create_plan(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PlanBody>,
) -> Response {
    let situation = body.situation.trim();
    if situation.is_empty() {
        return error_400("situation must not be empty".to_string());
    }
    let top_k = body
        .top_k
        .unwrap_or(DEFAULT_SEARCH_K)
        .clamp(1, MAX_SEARCH_K);
    match state
        .brain
        .plan_decision_tree(
            &planner_model(&state, &body.model),
            situation,
            top_k,
            opencoder_core::message::now_ms(),
        )
        .await
    {
        Ok((plan, tree)) => (
            StatusCode::CREATED,
            Json(json!({ "ok": true, "plan": plan, "tree": tree })),
        )
            .into_response(),
        Err(e) => map_plan_error("plan decision tree", e),
    }
}

/// GET /api/brain/plans/:id — one persisted plan (record + parsed tree) or
/// 404. A record whose `tree_json` no longer parses is a 500 (storage
/// corruption, never a client fault).
pub async fn get_plan(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.store.get_brain_plan(&id).await {
        Ok(Some(plan)) => {
            match serde_json::from_str::<opencoder_brain::DecisionTree>(&plan.tree_json) {
                Ok(tree) => Json(json!({ "ok": true, "plan": plan, "tree": tree })).into_response(),
                Err(e) => error_500(format!("get brain plan {id}: stored tree is corrupt: {e}")),
            }
        }
        Ok(None) => error_404(&format!("brain plan not found: {id}")),
        Err(e) => error_500(format!("get brain plan: {e:#}")),
    }
}

/// POST /api/brain/dispatch — route one situation to a capability. With
/// `plan_id`: through that exact plan (unknown id → 404). Without: the
/// dynamic scheduler — reuse the newest cached plan for the situation
/// digest, minting one first when needed (`replan` forces a fresh plan).
pub async fn dispatch(
    State(state): State<Arc<AppState>>,
    Json(body): Json<DispatchBody>,
) -> Response {
    let situation = body.situation.trim();
    if situation.is_empty() {
        return error_400("situation must not be empty".to_string());
    }
    let top_k = body
        .top_k
        .unwrap_or(DEFAULT_SEARCH_K)
        .clamp(1, MAX_SEARCH_K);
    let plan_id = body
        .plan_id
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string);
    let model = planner_model(&state, &body.model);
    let outcome = match plan_id {
        Some(id) => state
            .brain
            .dispatch_decision_tree(&id, situation)
            .await
            .map(|(plan, outcome)| (plan.id, outcome, false)),
        None => state
            .brain
            .dispatch_or_plan(
                &model,
                situation,
                top_k,
                body.replan.unwrap_or(false),
                opencoder_core::message::now_ms(),
            )
            .await
            .map(|d| (d.record.id, d.outcome, d.planned_fresh)),
    };
    match outcome {
        Ok((plan_id, outcome, planned_fresh)) => Json(json!({
            "ok": true,
            "plan_id": plan_id,
            "capability_id": outcome.capability_id,
            "reason": outcome.reason,
            "path": outcome.path,
            "planned_fresh": planned_fresh,
        }))
        .into_response(),
        Err(e) => map_plan_error("dispatch brain plan", e),
    }
}

// ─── client fallbacks ──────────────────────────────────────────────────

/// Bail-only `ChatStream` for the degraded serve() path: when the LLM config
/// cannot be loaded or the client cannot be built, the web server must still
/// boot — every brain write/search then surfaces a clear 502, because the
/// runtime folds this bail into its typed `EmbeddingFailed` marker
/// (rendered "embedding failed: llm endpoint unavailable") instead of a
/// panic or a silent skip.
pub struct UnavailableClient;

impl ChatStream for UnavailableClient {
    fn chat_stream(&self, _req: ChatRequest) -> Result<mpsc::Receiver<LlmEvent>> {
        bail!("llm endpoint unavailable")
    }

    fn backend(&self) -> &'static str {
        "unavailable"
    }

    fn embed(&self, _texts: &[String], _model: &str) -> Result<Vec<Vec<f32>>> {
        bail!("llm endpoint unavailable")
    }
}

/// Brain runtime wired to the bail-only client (degraded mode): every
/// embed-dependent call fails as the typed `EmbeddingFailed` marker → 502.
pub fn degraded_brain(store: Arc<dyn Store>) -> opencoder_brain::Runtime {
    opencoder_brain::Runtime::new(
        store,
        Arc::new(UnavailableClient) as Arc<dyn ChatStream>,
        "unavailable",
    )
}

/// Brain runtime wired to the deterministic mock embedder — the one-liner
/// every `AppState` literal in tests uses (same model id everywhere so vector
/// rows written by one test helper stay searchable by another).
pub fn mock_brain(store: Arc<dyn Store>) -> opencoder_brain::Runtime {
    opencoder_brain::Runtime::new(
        store,
        Arc::new(opencoder_llm::MockChatClient::new()),
        "mock-embed",
    )
}
