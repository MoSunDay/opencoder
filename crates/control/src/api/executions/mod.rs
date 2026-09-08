use super::response;
use crate::AppState;
use axum::{
    extract::{Path, State},
    response::Response,
    Json,
};
use opencoder_core::{fleet::*, message::now_ms};
use serde_json::json;
use std::sync::Arc;

pub async fn submit(state: &Arc<AppState>, request: CreateExecution) -> RpcReply {
    if request.kind == ExecutionKind::System {
        return RpcReply::error(
            400,
            "system team execution is retired; use explicit node maintenance",
        );
    }
    if request.kind == ExecutionKind::Project
        && request.id != format!("project-{}", request.target.as_deref().unwrap_or(""))
    {
        return RpcReply::error(
            400,
            "project execution id must be project-<todo id> to preserve plan/act affinity",
        );
    }
    if let Err(error) = request.validate() {
        return RpcReply::error(400, error);
    }
    match submit_inner(state, request).await {
        Ok(reply) => reply,
        Err(error) => RpcReply::error(500, format!("submit execution: {error:#}")),
    }
}

async fn submit_inner(state: &Arc<AppState>, request: CreateExecution) -> anyhow::Result<RpcReply> {
    let (_permit, assignment) = {
        let _gate = state.placement.lock().await;
        let permit = match state.admission.enter().await {
            Ok(permit) => permit,
            Err(error) => return Ok(RpcReply::error(503, error)),
        };
        if let Some(index) = state.fleet.index(&request.id).await? {
            if index.kind != request.kind {
                return Ok(RpcReply::error(
                    409,
                    "execution id is already assigned to another kind",
                ));
            }
            if request
                .node_id
                .as_deref()
                .is_some_and(|node| node != index.node_id)
            {
                return Ok(RpcReply::error(
                    409,
                    "execution is already assigned to another node",
                ));
            }
            // The node compares the original request and returns its durable
            // acceptance. A newer definition must not replace its snapshot.
            state.hub.reserve(&index).await;
            let assignment = Assignment {
                index,
                request,
                definition: None,
            };
            (permit, assignment)
        } else {
            let definition = match super::catalog::resolve(state, &request).await {
                Ok(definition) => definition,
                Err(reply) => return Ok(reply),
            };
            let mut nodes = Vec::new();
            for node in state.hub.views().await {
                if state.admission.node_allowed(&node).await {
                    nodes.push(node);
                }
            }
            let Some(node) =
                select_node(&nodes, request.kind, request.node_id.as_deref(), now_ms())
            else {
                return Ok(RpcReply::error(
                    503,
                    "no eligible online node with capacity for this execution",
                ));
            };
            let index = ExecutionIndex {
                id: request.id.clone(),
                created_at: now_ms(),
                kind: request.kind,
                node_id: node.registration.id.clone(),
                status: ExecutionStatus::Pending,
            };
            state.fleet.put_index(&index).await?;
            state.hub.reserve(&index).await;
            let assignment = Assignment {
                index,
                request,
                definition,
            };
            (permit, assignment)
        }
    };
    let index = assignment.index.clone();
    let mut reply = state
        .hub
        .call(
            &index.node_id,
            NodeOperation::Create {
                assignment: assignment.clone(),
            },
        )
        .await;
    if reply.status == 428 {
        let definition = match super::catalog::resolve(state, &assignment.request).await {
            Ok(definition) => definition,
            Err(reply) => return Ok(reply),
        };
        state.hub.reserve(&index).await;
        reply = state
            .hub
            .call(
                &index.node_id,
                NodeOperation::Create {
                    assignment: Assignment {
                        definition,
                        ..assignment
                    },
                },
            )
            .await;
    }
    if (200..300).contains(&reply.status) {
        let accepted: ExecutionIndex = serde_json::from_value(reply.body.clone())?;
        if accepted.id != index.id
            || accepted.node_id != index.node_id
            || accepted.created_at != index.created_at
            || accepted.kind != index.kind
        {
            anyhow::bail!("invalid node acceptance");
        }
        return Ok(RpcReply {
            status: 202,
            body: {
                let mut body = serde_json::to_value(accepted)?;
                if index.kind == ExecutionKind::Project {
                    if let Some(id) = reply.body.get("run_id") {
                        body["run_id"] = id.clone();
                    }
                }
                body
            },
        });
    }
    // The socket settles actual replies and complete reports own status. A
    // timeout/disconnect retains Pending because the node may have accepted it.
    Ok(reply)
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateExecution>,
) -> Response {
    response(submit(&state, request).await)
}

mod paging;
pub use paging::{
    detail_field, event_payload, events_page, inspect, list, messages, project_runs, team_turns,
    todo_items, ProjectRunsQuery,
};
pub async fn command(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(command): Json<ExecutionCommand>,
) -> Response {
    response(dispatch_command(&state, &id, command).await)
}

/// Every public control route uses the same authoritative Plan snapshot.
/// The index retains ownership; the node remains the admission authority.
pub async fn dispatch_command(
    state: &Arc<AppState>,
    id: &str,
    mut command: ExecutionCommand,
) -> RpcReply {
    match state.fleet.index(id).await {
        Ok(Some(index))
            if index.kind == ExecutionKind::System
                && !matches!(command.action.as_str(), "cancel" | "interrupt") =>
        {
            return RpcReply::error(
                400,
                "historical system executions only support cancel or interrupt",
            );
        }
        Ok(_) => {}
        Err(error) => return RpcReply::error(500, format!("index: {error:#}")),
    }
    if matches!(command.action.as_str(), "plan" | "execute") {
        let Some(todo) = id.strip_prefix("project-").filter(|_| valid_id(id)) else {
            return RpcReply::error(400, "plan/execute requires project execution");
        };
        if let Some(run_id) = command.input.get("run_id") {
            if !run_id
                .as_str()
                .is_some_and(|id| id.starts_with("prun-") && valid_id(id))
            {
                return RpcReply::error(400, "invalid project run id");
            }
            let receipt = command_id(
                state,
                id,
                ExecutionCommand {
                    action: "project-receipt".into(),
                    input: json!({"action":command.action,"input":command.input}),
                },
            )
            .await;
            if receipt.status != 404 {
                return receipt;
            }
        }
        command.input =
            match super::project::brain_preresolve(state, todo, &command.action, command.input)
                .await
            {
                Ok(input) => input,
                Err(reply) => return reply,
            };
        {
            let request = CreateExecution {
                id: id.into(),
                kind: ExecutionKind::Project,
                target: Some(todo.into()),
                input: serde_json::Value::Null,
                node_id: None,
            };
            match super::catalog::resolve(state, &request).await {
                Ok(Some(snapshot)) => {
                    if command.input.is_null() {
                        command.input = json!({});
                    }
                    command.input["snapshot"] = snapshot;
                }
                Ok(None) => return RpcReply::error(500, "project snapshot missing"),
                Err(reply) => return reply,
            }
        }
    }
    command_id(state, id, command).await
}
pub async fn inspect_id(state: &AppState, id: &str) -> RpcReply {
    for_id(state, id, |execution| NodeOperation::Inspect { execution }).await
}
pub async fn command_id(state: &AppState, id: &str, command: ExecutionCommand) -> RpcReply {
    let _permit = if crate::admission::command_requires_admission(&command) {
        let _placement = state.placement.lock().await;
        match state.admission.enter().await {
            Ok(permit) => Some(permit),
            Err(error) => return RpcReply::error(503, error),
        }
    } else {
        None
    };
    for_id(state, id, |execution| NodeOperation::Command {
        execution,
        command,
    })
    .await
}
pub async fn events_id(state: &AppState, id: &str, after: i64) -> RpcReply {
    for_id(state, id, |execution| NodeOperation::Events {
        execution,
        after,
    })
    .await
}
pub async fn messages_id(state: &AppState, id: &str, cursor: MessageCursor) -> RpcReply {
    for_id(state, id, |execution| NodeOperation::Messages {
        execution,
        cursor,
    })
    .await
}
pub async fn event_payload_id(state: &AppState, id: &str, seq: i64, offset: u64) -> RpcReply {
    for_id(state, id, |execution| NodeOperation::EventPayload {
        request: EventPayloadRequest {
            execution,
            seq,
            offset,
        },
    })
    .await
}
pub(super) async fn for_id(
    state: &AppState,
    id: &str,
    operation: impl FnOnce(ExecutionRef) -> NodeOperation,
) -> RpcReply {
    match state.fleet.index(id).await {
        Ok(Some(index)) => {
            let node_id = index.node_id.clone();
            state
                .hub
                .call(&node_id, operation(index.execution_ref()))
                .await
        }
        Ok(None) => RpcReply::error(404, "execution id not found"),
        Err(error) => RpcReply::error(500, format!("index: {error:#}")),
    }
}
