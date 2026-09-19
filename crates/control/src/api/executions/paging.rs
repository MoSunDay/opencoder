use super::super::{error_400, error_500, response};
use crate::AppState;
use axum::{
    extract::{Path, Query, State},
    response::Response,
};
use opencoder_core::fleet::*;
use opencoder_store::fleet::handoff::ExecutionNames;
use serde::Deserialize;
use serde_json::{json, Value};
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
        Ok(page) => match named_page(&state, page).await {
            Ok(body) => response(RpcReply::ok(body)),
            Err(error) => error_500(error),
        },
        Err(error) => error_500(format!("index: {error:#}")),
    }
}

/// Kind-aware display name from the dispatch-time snapshot. The five-field
/// index DTO stays protocol-locked; the name is lifted at the JSON layer only
/// (same contract as compat `dag_view`).
fn display_name(kind: ExecutionKind, names: &ExecutionNames) -> Option<String> {
    match kind {
        // Team/Dag prefer the frozen definition name; the target is only a
        // fallback for assignments that predate the definition snapshot.
        ExecutionKind::Team => names
            .definition_name
            .clone()
            .or_else(|| names.target.clone()),
        ExecutionKind::Dag => names
            .spec_name
            .clone()
            .or_else(|| names.definition_name.clone())
            .or_else(|| names.target.clone()),
        // The target is the name itself (agent name, template/version, todo id).
        ExecutionKind::Agent
        | ExecutionKind::Todos
        | ExecutionKind::Project
        | ExecutionKind::Maintenance
        | ExecutionKind::Operator => names.target.clone(),
        // Brain/System carry no kind-unique target id.
        ExecutionKind::Brain | ExecutionKind::System => None,
    }
}

async fn named_page(
    state: &AppState,
    page: ExecutionPage<ExecutionIndex>,
) -> Result<Value, String> {
    let mut body = serde_json::to_value(&page).map_err(|error| error.to_string())?;
    let rows = body
        .get_mut("executions")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "invalid index page".to_string())?;
    let ids: Vec<String> = rows
        .iter()
        .filter_map(|row| row["id"].as_str().map(str::to_owned))
        .collect();
    let names = state
        .fleet
        .execution_names(&ids)
        .await
        .map_err(|error| format!("execution names: {error:#}"))?;
    for row in rows.iter_mut() {
        let id = row["id"].as_str().unwrap_or_default();
        let Ok(kind) = serde_json::from_value::<ExecutionKind>(row["kind"].clone()) else {
            continue;
        };
        if let Some(name) = names.get(id).and_then(|names| display_name(kind, names)) {
            row["name"] = json!(name);
        }
    }
    Ok(body)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn names(target: Option<&str>, definition: Option<&str>, spec: Option<&str>) -> ExecutionNames {
        ExecutionNames {
            target: target.map(str::to_owned),
            definition_name: definition.map(str::to_owned),
            spec_name: spec.map(str::to_owned),
        }
    }

    #[test]
    fn team_and_dag_prefer_the_definition_snapshot_then_fall_back_to_target() {
        // Team: top-level definition name wins over a stale target.
        assert_eq!(
            display_name(ExecutionKind::Team, &names(Some("old"), Some("demo"), None)),
            Some("demo".into())
        );
        // Dag: spec name first, then top-level name, then the target.
        assert_eq!(
            display_name(
                ExecutionKind::Dag,
                &names(Some("dag-x"), Some("etl"), Some("etl-v2"))
            ),
            Some("etl-v2".into())
        );
        assert_eq!(
            display_name(ExecutionKind::Dag, &names(Some("dag-x"), None, None)),
            Some("dag-x".into())
        );
        // A spec-only snapshot (inline definitions) still names the row.
        assert_eq!(
            display_name(ExecutionKind::Dag, &names(None, None, Some("legacy"))),
            Some("legacy".into())
        );
    }

    #[test]
    fn target_named_kinds_ignore_the_definition_snapshot() {
        // Todos would otherwise show a template-internal name instead of the
        // template/version the operator launched with.
        assert_eq!(
            display_name(
                ExecutionKind::Todos,
                &names(Some("review/v2"), Some("模板内部名"), None)
            ),
            Some("review/v2".into())
        );
        assert_eq!(
            display_name(ExecutionKind::Agent, &names(Some("coder-x"), None, None)),
            Some("coder-x".into())
        );
        assert_eq!(
            display_name(
                ExecutionKind::Project,
                &names(Some("todo-9"), Some("x"), None)
            ),
            Some("todo-9".into())
        );
        assert_eq!(
            display_name(ExecutionKind::Maintenance, &names(Some("fix"), None, None)),
            Some("fix".into())
        );
    }

    #[test]
    fn brain_system_and_incomplete_snapshots_stay_unnamed() {
        assert_eq!(
            display_name(ExecutionKind::Brain, &names(Some("x"), Some("y"), None)),
            None
        );
        assert_eq!(
            display_name(ExecutionKind::System, &names(None, None, None)),
            None
        );
        // An agent assignment without a target names nothing (SPA renders `-`).
        assert_eq!(
            display_name(ExecutionKind::Agent, &names(None, None, None)),
            None
        );
    }
}
