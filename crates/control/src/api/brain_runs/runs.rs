use crate::{
    api::{error_400, error_500, response},
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    response::Response,
    Json,
};
use opencoder_core::{
    brain::layered::{LAYERED_MIGRATION, LAYERED_SCHEMA_VERSION},
    fleet::*,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub async fn create(State(state): State<Arc<AppState>>, Json(value): Json<Value>) -> Response {
    if value["schema_version"] != LAYERED_SCHEMA_VERSION {
        return response(RpcReply::error(409, LAYERED_MIGRATION));
    }
    super::v4::create(State(state), Json(value)).await
}
pub async fn require_v4(state: &AppState, id: &str) -> Result<(), RpcReply> {
    match state.fleet.assignment(id).await {
        Ok(Some(a)) if matches!(a.request.input["schema_version"].as_u64(), Some(4..=7)) => Ok(()),
        Ok(_) => Err(RpcReply::error(409, LAYERED_MIGRATION)),
        Err(error) => Err(RpcReply::error(500, error.to_string())),
    }
}
pub use require_v4 as require_brain;

pub async fn list(State(state): State<Arc<AppState>>, Query(page): Query<Page>) -> Response {
    let cursor = match (page.cursor_created_at, page.cursor_id) {
        (None, None) => None,
        (Some(created_at), Some(id)) if valid_id(&id) => Some(ExecutionCursor { created_at, id }),
        _ => {
            return error_400("cursor_created_at and valid cursor_id are required together".into())
        }
    };
    match state
        .fleet
        .indexes_page(None, Some(ExecutionKind::Brain), cursor.as_ref(), 100)
        .await
    {
        Ok(page) => response(RpcReply::ok(
            json!({"runs":page.executions,"next_cursor":page.next_cursor}),
        )),
        Err(e) => error_500(e.to_string()),
    }
}
pub async fn call(state: &Arc<AppState>, id: &str, action: &str, input: Value) -> RpcReply {
    let index = match state.fleet.index(id).await {
        Ok(Some(i)) if i.kind == ExecutionKind::Brain => i,
        Ok(_) => return RpcReply::error(404, "brain run not found"),
        Err(e) => return RpcReply::error(500, e.to_string()),
    };
    state
        .hub
        .call(
            &index.node_id,
            NodeOperation::Brain {
                execution: index.execution_ref(),
                action: action.into(),
                input,
            },
        )
        .await
}
#[derive(Default, Deserialize)]
pub struct Page {
    pub cursor_created_at: Option<i64>,
    pub cursor_id: Option<String>,
    pub after: Option<u64>,
    pub limit: Option<u32>,
}
pub async fn snapshot(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    if let Err(reply) = require_v4(&state, &id).await {
        return response(reply);
    }
    super::v4::snapshot(State(state), Path(id)).await
}
pub async fn events(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(page): Query<Page>,
) -> Response {
    if let Err(reply) = require_v4(&state, &id).await {
        return response(reply);
    }
    super::v4::events(
        State(state),
        Path(id),
        Query(super::v4::Page {
            after: page.after,
            limit: page.limit,
        }),
    )
    .await
}
pub async fn command(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(command): Json<ExecutionCommand>,
) -> Response {
    if let Err(reply) = require_v4(&state, &id).await {
        return response(reply);
    }
    super::v4::command(State(state), Path(id), Json(command)).await
}
