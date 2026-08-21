//! Pending-input inspection/management (TUI queue-panel parity over HTTP):
//! list what is queued/steered but not yet consumed, delete one, swap order.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use opencoder_store::Delivery;

use crate::AppState;

#[derive(Deserialize, Default)]
pub struct InputsQuery {
    pub delivery: Option<String>,
}

/// GET /api/sessions/:id/inputs?delivery=queue|steer (default steer).
///
/// Lists still-pending (unpromoted) inputs of that delivery. A missing
/// session row is not an error — an empty list is fine.
pub async fn list_inputs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<InputsQuery>,
) -> Response {
    let delivery = match q.delivery.as_deref() {
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
    let rows = match state.store.pending_inputs(&id, delivery).await {
        Ok(v) => v,
        Err(e) => return error_500(format!("pending_inputs: {e:#}")),
    };
    let inputs: Vec<_> = rows
        .iter()
        .map(|i| {
            json!({
                "seq": i.seq,
                "delivery": i.delivery.as_str(),
                "prompt": i.prompt,
                "admitted_seq": i.admitted_seq,
                "promoted_seq": i.promoted_seq,
                "images": i.images.len(),
            })
        })
        .collect();
    Json(json!({ "inputs": inputs })).into_response()
}

/// DELETE /api/sessions/:id/inputs/:seq — remove a pending input before the
/// drain consumes it (mirrors the TUI queue panel). 404 when the seq is not
/// currently pending in EITHER delivery (already consumed / wrong session).
pub async fn delete_input(
    State(state): State<Arc<AppState>>,
    Path((id, seq)): Path<(String, i64)>,
) -> Response {
    let store = state.store.clone();
    let is_pending =
        |rows: &Vec<opencoder_store::SessionInput>| rows.iter().any(|i| i.seq == Some(seq));
    let (steer, queue) = match (
        store.pending_inputs(&id, Delivery::Steer).await,
        store.pending_inputs(&id, Delivery::Queue).await,
    ) {
        (Ok(a), Ok(b)) => (is_pending(&a), is_pending(&b)),
        (Err(e), _) | (_, Err(e)) => return error_500(format!("pending_inputs: {e:#}")),
    };
    if !steer && !queue {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "error": format!("input {seq} not pending") })),
        )
            .into_response();
    }
    match store.delete_input(seq).await {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => error_500(format!("delete_input: {e:#}")),
    }
}

#[derive(Deserialize)]
pub struct ReorderBody {
    pub a: i64,
    pub b: i64,
}

/// POST /api/sessions/:id/inputs/reorder — swap the drain order of two
/// pending inputs by exchanging their `admitted_seq` (TUI queue-panel parity).
pub async fn reorder_inputs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<ReorderBody>,
) -> Response {
    match state.store.swap_input_order(&id, body.a, body.b).await {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => error_500(format!("swap_input_order: {e:#}")),
    }
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
