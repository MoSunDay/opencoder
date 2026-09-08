use super::super::{error_400, error_500, response};
use crate::AppState;
use axum::{
    extract::{Path, Query, State},
    response::Response,
};
use opencoder_core::fleet::*;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct ListQuery {
    pub node_id: Option<String>,
    pub kind: Option<String>,
    pub limit: Option<u32>,
    pub cursor_created_at: Option<i64>,
    pub cursor_id: Option<String>,
}
pub async fn list(State(state): State<Arc<AppState>>, Query(query): Query<ListQuery>) -> Response {
    let kind = match query
        .kind
        .map(|kind| serde_json::from_value::<ExecutionKind>(json!(kind)))
        .transpose()
    {
        Ok(kind) => kind,
        Err(_) => return response(RpcReply::error(400, "invalid execution kind")),
    };
    let limit = query.limit.unwrap_or(EXECUTION_PAGE_DEFAULT);
    if !(1..=EXECUTION_PAGE_MAX).contains(&limit) {
        return error_400(format!("limit must be between 1 and {EXECUTION_PAGE_MAX}"));
    }
    let cursor = match (query.cursor_created_at, query.cursor_id) {
        (None, None) => None,
        (Some(created_at), Some(id)) if valid_id(&id) => Some(ExecutionCursor { created_at, id }),
        _ => {
            return error_400(
                "cursor_created_at and a valid cursor_id are required together".into(),
            )
        }
    };
    match state
        .fleet
        .indexes_page(query.node_id.as_deref(), kind, cursor.as_ref(), limit)
        .await
    {
        Ok(page) => response(RpcReply::ok(json!(page))),
        Err(error) => error_500(format!("index: {error:#}")),
    }
}
pub async fn inspect(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    response(super::inspect_id(&state, &id).await)
}
#[derive(Deserialize)]
pub struct MessageQuery {
    pub seq: Option<i64>,
    pub offset: Option<u64>,
}
pub async fn messages(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<MessageQuery>,
) -> Response {
    let cursor = MessageCursor {
        seq: query.seq.unwrap_or(0),
        offset: query.offset.unwrap_or(0),
    };
    if cursor.seq < 0 || (cursor.seq == 0 && cursor.offset != 0) {
        return error_400("invalid message cursor".into());
    }
    response(super::messages_id(&state, &id, cursor).await)
}
#[derive(Deserialize)]
pub struct EventPayloadQuery {
    pub offset: Option<u64>,
}
pub async fn event_payload(
    State(state): State<Arc<AppState>>,
    Path((id, seq)): Path<(String, i64)>,
    Query(query): Query<EventPayloadQuery>,
) -> Response {
    if seq <= 0 {
        return error_400("event sequence must be positive".into());
    }
    response(super::event_payload_id(&state, &id, seq, query.offset.unwrap_or(0)).await)
}
#[derive(Deserialize)]
pub struct DetailFieldQuery {
    pub field: String,
    pub offset: Option<u64>,
}
pub async fn detail_field(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<DetailFieldQuery>,
) -> Response {
    if query.field.is_empty()
        || query.field.len() > 200
        || !query
            .field
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return error_400("invalid detail field".into());
    }
    response(
        super::for_id(&state, &id, |execution| NodeOperation::DetailField {
            request: DetailFieldRequest {
                execution,
                field: query.field,
                offset: query.offset.unwrap_or(0),
            },
        })
        .await,
    )
}
#[derive(Deserialize)]
pub struct TodoItemsQuery {
    pub after_ordinal: Option<i64>,
}
pub async fn todo_items(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<TodoItemsQuery>,
) -> Response {
    if query.after_ordinal.is_some_and(|value| value < 0) {
        return error_400("invalid TODO item cursor".into());
    }
    response(
        super::for_id(&state, &id, |execution| NodeOperation::TodoItems {
            execution,
            after_ordinal: query.after_ordinal,
        })
        .await,
    )
}
#[derive(Deserialize)]
pub struct ProjectRunsQuery {
    pub before_version: Option<i64>,
}
pub async fn project_runs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<ProjectRunsQuery>,
) -> Response {
    if query.before_version.is_some_and(|value| value <= 0) {
        return error_400("invalid project run cursor".into());
    }
    response(
        super::for_id(&state, &id, |execution| NodeOperation::ProjectRuns {
            execution,
            before_version: query.before_version,
        })
        .await,
    )
}
#[derive(Deserialize)]
pub struct TeamTurnsQuery {
    pub after_turn: Option<u32>,
}
pub async fn team_turns(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<TeamTurnsQuery>,
) -> Response {
    response(
        super::for_id(&state, &id, |execution| NodeOperation::TeamTurns {
            execution,
            after_turn: query.after_turn.unwrap_or(0),
        })
        .await,
    )
}

#[derive(Deserialize)]
pub struct EventsPageQuery {
    #[serde(default)]
    pub after: i64,
}
pub async fn events_page(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<EventsPageQuery>,
) -> Response {
    if query.after < 0 {
        return error_400("invalid event cursor".into());
    }
    response(super::events_id(&state, &id, query.after).await)
}
