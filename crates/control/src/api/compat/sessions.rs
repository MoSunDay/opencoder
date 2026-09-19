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
            ExecutionKind::Operator,
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
        .unwrap_or_else(|| format!("operator-{}", ulid::Ulid::new()));
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
                kind: ExecutionKind::Operator,
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
pub async fn dialogs(
    State(state): State<Arc<AppState>>,
    Path(node): Path<String>,
    Query(query): Query<super::super::session::SessionKindQuery>,
) -> Response {
    let kind = match super::super::session::requested_query_kind(query.kind.as_deref()) {
        Ok(kind) => kind,
        Err(error) => return super::super::error_400(error),
    };
    match super::super::session::summaries(&state, Some(&node), kind).await {
        Ok(rows) => response(RpcReply::ok(json!({
            "dialogs": rows
                .into_iter()
                .map(|r| {
                    json!({
                        "session_id": r["id"],
                        "title": r["title"],
                        "first_created_at": r["created_at"],
                        "last_created_at": r["updated_at"],
                        "status": r["status"],
                        "kind": r["kind"],
                        "execution_ref": r["execution_ref"],
                        "detail_error": r["detail_error"],
                    })
                })
                .collect::<Vec<_>>(),
        }))),
        Err(e) => response(RpcReply::error(500, e.to_string())),
    }
}
/// DELETE /api/nodes/:id/dialogs — bulk-clear the node's console dialogs.
/// Dialog rows live in the node's own runtime store, so the control plane
/// decides what is safe to drop from the selected execution index and forwards
/// a `dialogs_clear` maintenance command to the node: executions still
/// pending/running/cancelling (and interrupted, i.e. resumable) survive and
/// are reported as `skipped`; only the requested Operator or Agent lane is
/// deleted, so the two conversation lists never affect one another.
pub async fn clear_dialogs(
    State(state): State<Arc<AppState>>,
    Path(node): Path<String>,
    Query(query): Query<super::super::session::SessionKindQuery>,
) -> Response {
    let known = state
        .fleet
        .nodes()
        .await
        .map(|nodes| nodes.iter().any(|n| n.id == node))
        .unwrap_or(false);
    if !known {
        return response(RpcReply::error(404, "node not found"));
    }
    let kind = match super::super::session::requested_query_kind(query.kind.as_deref()) {
        Ok(kind) => kind,
        Err(error) => return super::super::error_400(error),
    };
    let indexes = match state.fleet.indexes(Some(&node), Some(kind), 500).await {
        Ok(rows) => rows,
        Err(e) => return response(RpcReply::error(500, e.to_string())),
    };
    let droppable = |status: &ExecutionStatus| {
        matches!(
            status,
            ExecutionStatus::Idle
                | ExecutionStatus::Done
                | ExecutionStatus::Error
                | ExecutionStatus::Cancelled
        )
    };
    let drop_ids: Vec<String> = indexes
        .iter()
        .filter(|ix| droppable(&ix.status))
        .map(|ix| ix.id.clone())
        .collect();
    let mut skipped: Vec<String> = indexes
        .iter()
        .filter(|ix| !droppable(&ix.status))
        .map(|ix| ix.id.clone())
        .collect();
    let reply = state
        .hub
        .call(
            &node,
            NodeOperation::Maintenance {
                command: ExecutionCommand {
                    action: "dialogs_clear".into(),
                    input: json!({ "sessions": drop_ids }),
                },
            },
        )
        .await;
    if reply.status >= 300 {
        return response(reply);
    }
    if let Some(extra) = reply.body["skipped"].as_array() {
        for value in extra {
            if let Some(id) = value.as_str() {
                if !skipped.iter().any(|known| known == id) {
                    skipped.push(id.to_owned());
                }
            }
        }
    }
    if !drop_ids.is_empty() {
        if let Err(e) = state
            .fleet
            .delete_terminal_indexes(&node, kind, &drop_ids)
            .await
        {
            return response(RpcReply::error(500, format!("delete_indexes: {e:#}")));
        }
    }
    let removed = reply
        .body
        .get("removed")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    response(RpcReply::ok(
        json!({"ok": true, "removed": removed, "skipped": skipped}),
    ))
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
