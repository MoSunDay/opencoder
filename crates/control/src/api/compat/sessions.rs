use crate::{
    api::{executions, response},
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    response::Response,
    Json,
};
use opencoder_core::fleet::*;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Deserialize)]
pub struct NodeQuery {
    node_id: Option<String>,
}
async fn metadata(state: Arc<AppState>, query: NodeQuery, action: &str) -> Response {
    let nodes = state.hub.views().await;
    let node = query.node_id.or_else(|| {
        select_node(
            &nodes,
            ExecutionKind::Agent,
            None,
            opencoder_core::message::now_ms(),
        )
        .map(|n| n.registration.id.clone())
    });
    let Some(node) = node else {
        return response(RpcReply::error(
            503,
            "no online node for configuration query",
        ));
    };
    response(
        state
            .hub
            .call(
                &node,
                NodeOperation::Maintenance {
                    command: ExecutionCommand {
                        action: action.into(),
                        input: Value::Null,
                    },
                },
            )
            .await,
    )
}
pub async fn models(
    State(state): State<Arc<AppState>>,
    Query(query): Query<NodeQuery>,
) -> Response {
    metadata(state, query, "models").await
}
pub async fn skills(
    State(state): State<Arc<AppState>>,
    Query(query): Query<NodeQuery>,
) -> Response {
    metadata(state, query, "skills").await
}
pub async fn task(
    State(state): State<Arc<AppState>>,
    Path(node): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let sid = body["session_id"].as_str();
    let id = sid
        .or(body["id"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("agent-{}", ulid::Ulid::new()));
    let reply = if sid.is_some() {
        if !matches!(state.fleet.index(&id).await,Ok(Some(index)) if index.node_id==node) {
            return response(RpcReply::error(
                409,
                "conversation belongs to a different node",
            ));
        }
        executions::command_id(
            &state,
            &id,
            ExecutionCommand {
                action: "prompt".into(),
                input: body,
            },
        )
        .await
    } else {
        executions::submit(
            &state,
            CreateExecution {
                id: id.clone(),
                kind: ExecutionKind::Agent,
                target: body["agent"].as_str().map(str::to_owned),
                input: body,
                node_id: Some(node.clone()),
            },
        )
        .await
    };
    if reply.status >= 300 {
        return response(reply);
    }
    response(RpcReply::ok(
        json!({"task_id":id,"session_id":id,"node_id":node}),
    ))
}
pub async fn dialogs(State(state): State<Arc<AppState>>, Path(node): Path<String>) -> Response {
    match super::super::session::summaries(&state, Some(&node)).await {
        Ok(rows) => response(RpcReply::ok(
            json!({"dialogs":rows.into_iter().map(|r|json!({"session_id":r["id"],"title":r["title"],"first_created_at":r["created_at"],"last_created_at":r["updated_at"],"status":r["status"],"detail_error":r["detail_error"]})).collect::<Vec<_>>()}),
        )),
        Err(e) => response(RpcReply::error(500, e.to_string())),
    }
}
pub async fn owner(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.fleet.index(&id).await {
        Ok(Some(index)) => response(RpcReply::ok(
            json!({"task":{"task_id":id,"session_id":id,"node_id":index.node_id,"status":index.status},"node_id":index.node_id,"task_id":id}),
        )),
        Ok(None) => response(RpcReply::error(404, "execution not found")),
        Err(e) => response(RpcReply::error(500, e.to_string())),
    }
}
pub async fn cancel(
    State(state): State<Arc<AppState>>,
    Path((node, id)): Path<(String, String)>,
) -> Response {
    if !matches!(state.fleet.index(&id).await,Ok(Some(index)) if index.node_id==node) {
        return response(RpcReply::error(
            409,
            "execution belongs to a different node",
        ));
    }
    response(
        executions::command_id(
            &state,
            &id,
            ExecutionCommand {
                action: "cancel".into(),
                input: Value::Null,
            },
        )
        .await,
    )
}
