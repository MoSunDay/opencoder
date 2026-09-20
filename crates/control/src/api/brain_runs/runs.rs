use crate::{
    api::{error_400, error_500, response},
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    response::Response,
    Json,
};
use opencoder_core::{brain::*, fleet::*};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

/// New runs must opt into the reference-only scheduler protocol explicitly.
pub async fn create(State(state): State<Arc<AppState>>, Json(value): Json<Value>) -> Response {
    if value.get("schema_version").and_then(Value::as_u64) != Some(3) {
        return response(RpcReply::error(409, SCHEDULER_MIGRATION));
    }
    super::v3::create(State(state), Json(value)).await
}

/// Used by every public write route, including generic execution commands.
/// An absent assignment is historical/unknown, never permission to use v2 writes.
pub async fn require_v3(state: &AppState, id: &str) -> Result<(), RpcReply> {
    match state.fleet.assignment(id).await {
        Ok(Some(a)) if a.request.input["schema_version"] == 3 => Ok(()),
        Ok(_) => Err(RpcReply::error(409, SCHEDULER_MIGRATION)),
        Err(error) => Err(RpcReply::error(500, error.to_string())),
    }
}
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
    pub step: Option<String>,
    pub offset: Option<u64>,
    pub after: Option<i64>,
    pub limit: Option<u32>,
}
pub async fn snapshot(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(page): Query<Page>,
) -> Response {
    if state
        .fleet
        .assignment(&id)
        .await
        .ok()
        .flatten()
        .is_some_and(|a| a.request.input["schema_version"] == 3)
    {
        return super::v3::snapshot(State(state), Path(id)).await;
    }
    response(
        call(
            &state,
            &id,
            "snapshot",
            json!({"offset":page.offset.unwrap_or(0)}),
        )
        .await,
    )
}
pub async fn context(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    response(call(&state, &id, "context", json!({})).await)
}
pub async fn actions(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(page): Query<Page>,
) -> Response {
    response(
        call(
            &state,
            &id,
            "actions",
            json!({"offset":page.offset.unwrap_or(0)}),
        )
        .await,
    )
}
pub async fn events(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(page): Query<Page>,
) -> Response {
    if page.after.is_some_and(|after| after < 0) {
        return error_400("event cursor must be nonnegative".into());
    }
    if state
        .fleet
        .assignment(&id)
        .await
        .ok()
        .flatten()
        .is_some_and(|a| a.request.input["schema_version"] == 3)
    {
        let page = super::v3::Page {
            after: page.after.map(|v| v as u64),
            limit: page.limit,
        };
        return super::v3::events(State(state), Path(id), Query(page)).await;
    }
    response(
        call(
            &state,
            &id,
            "events",
            json!({"after":page.after.unwrap_or(0)}),
        )
        .await,
    )
}
pub async fn instance(
    State(state): State<Arc<AppState>>,
    Path((id, instance)): Path<(String, String)>,
) -> Response {
    response(call(&state, &id, "instance", json!({"id":instance})).await)
}
pub async fn input() -> Response {
    response(RpcReply::error(409, SCHEDULER_MIGRATION))
}
pub async fn command(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(command): Json<ExecutionCommand>,
) -> Response {
    if let Err(reply) = require_v3(&state, &id).await {
        return response(reply);
    }
    super::v3::command(State(state), Path(id), Json(command)).await
}

pub async fn instances(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(page): Query<Page>,
) -> Response {
    response(
        call(
            &state,
            &id,
            "instances",
            json!({"step":page.step,"offset":page.offset.unwrap_or(0)}),
        )
        .await,
    )
}
