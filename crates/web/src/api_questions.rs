//! `question` tool bridge for non-SSE frontends.
//!
//! A poll-based web UI (no SSE connection) can still answer a model question
//! mid-turn: GET attaches the hub (so the tool waits instead of falling back
//! to NO_LISTENER_REPLY) and lists what is open; POST answer/skip resolves
//! the blocked tool call. No running-gate by design — answering WHILE the
//! drain runs is the whole point (the tool is blocked inside the turn).

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::handle_questions::get_or_create_handle;
use crate::AppState;

/// GET /api/sessions/:id/questions — open questions (200, always).
///
/// The get-or-create + attach means even a pure polling frontend enables
/// waiting: the first poll flips the hub to attached before the tool checks
/// it (or the next drain's registry picks the same hub up).
pub async fn list_questions(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let handle = get_or_create_handle(&state.handles, &id).await;
    handle.question_hub.attach();
    let questions: Vec<_> = handle
        .question_hub
        .waiting_questions()
        .into_iter()
        .map(|(call_id, p)| json!({ "id": call_id, "question": p.question, "options": p.options }))
        .collect();
    Json(json!({ "questions": questions })).into_response()
}

#[derive(Deserialize)]
pub struct AnswerBody {
    pub answer: String,
}

/// POST /api/sessions/:id/questions/:call_id/answer — resolve the question.
///
/// The waiting pre-check makes an unknown/already-answered id a deterministic
/// 404 for HTTP callers: unlike the TUI (which may answer in the
/// ToolStart-before-register race and relies on `resolve`'s early parking),
/// a polling web frontend can only ever see ids that are currently waiting.
/// If pre-check passes but `resolve` returns false (the oneshot receiver was
/// dropped — tool side already gone), that is also a 404.
pub async fn answer_question(
    State(state): State<Arc<AppState>>,
    Path((id, call_id)): Path<(String, String)>,
    body: Option<Json<AnswerBody>>,
) -> Response {
    let answer = match body {
        Some(Json(b)) => b.answer,
        None => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(json!({ "ok": false, "error": "missing body field: answer" })),
            )
                .into_response()
        }
    };
    if answer.trim().is_empty() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "answer must not be empty" })),
        )
            .into_response();
    }
    let handle = get_or_create_handle(&state.handles, &id).await;
    let not_waiting = !handle
        .question_hub
        .waiting_questions()
        .iter()
        .any(|(cid, _)| cid == &call_id);
    if not_waiting || !handle.question_hub.resolve(&call_id, answer) {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "error": "question not waiting" })),
        )
            .into_response();
    }
    Json(json!({ "ok": true })).into_response()
}

/// POST /api/sessions/:id/questions/:call_id/skip — abandon the question so
/// the tool resolves to the fixed SKIPPED_REPLY and the turn completes.
pub async fn skip_question(
    State(state): State<Arc<AppState>>,
    Path((id, call_id)): Path<(String, String)>,
) -> Response {
    let handle = get_or_create_handle(&state.handles, &id).await;
    let waiting = handle.question_hub.waiting_questions();
    if !waiting.iter().any(|(cid, _)| cid == &call_id) {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "error": "question not waiting" })),
        )
            .into_response();
    }
    handle.question_hub.abandon(&call_id);
    Json(json!({ "ok": true })).into_response()
}
