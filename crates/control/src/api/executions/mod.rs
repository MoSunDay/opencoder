use super::response;
use crate::AppState;
use axum::{
    extract::{Path, State},
    response::Response,
    Json,
};
use opencoder_core::fleet::*;
use serde_json::json;
use std::sync::Arc;

mod submit;
pub use submit::submit;

pub async fn create(
    State(state): State<Arc<AppState>>,
    identity: Option<axum::Extension<opencoder_core::identity::Identity>>,
    Json(request): Json<CreateExecution>,
) -> Response {
    if identity
        .as_ref()
        .map(|axum::Extension(i)| i)
        .is_some_and(|i| !i.is_admin() && request.kind != ExecutionKind::Operator)
    {
        return response(RpcReply::error(
            403,
            "non-admin roles may only submit operator executions",
        ));
    }
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
    identity: Option<axum::Extension<opencoder_core::identity::Identity>>,
    Json(command): Json<ExecutionCommand>,
) -> Response {
    let identity = identity.map(|axum::Extension(i)| i);
    response(dispatch_command_as(&state, &id, command, identity.as_ref()).await)
}

/// Every public control route uses the same authoritative Plan snapshot.
/// The index retains ownership; the node remains the admission authority.
pub async fn dispatch_command(
    state: &Arc<AppState>,
    id: &str,
    command: ExecutionCommand,
) -> RpcReply {
    dispatch_command_as(state, id, command, None).await
}

/// [`dispatch_command`] with the caller's identity: non-admins may command
/// only operator executions (the role gate already limited them to this
/// endpoint family).
pub async fn dispatch_command_as(
    state: &Arc<AppState>,
    id: &str,
    mut command: ExecutionCommand,
    identity: Option<&opencoder_core::identity::Identity>,
) -> RpcReply {
    match state.fleet.index(id).await {
        Ok(Some(index))
            if identity.is_some_and(|i| !i.is_admin() && index.kind != ExecutionKind::Operator) =>
        {
            return RpcReply::error(403, "non-admin roles may only command operator executions");
        }
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

pub async fn receipt(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.fleet.receipt("execution", &id).await {
        Ok(Some(receipt)) => response(RpcReply::ok(json!({"id":id,"phase":receipt.phase,
            "receipt": if matches!(receipt.phase.as_str(), "accepted" | "rejected") { receipt.payload } else { serde_json::Value::Null }}))),
        Ok(None) => response(RpcReply::error(404, "request receipt not found")),
        Err(error) => response(RpcReply::error(500, error.to_string())),
    }
}
pub async fn command_id(state: &AppState, id: &str, command: ExecutionCommand) -> RpcReply {
    let _process_lock = match state.fleet.request_lock("brain-control", id).await {
        Ok(lock) => lock,
        Err(error) => return RpcReply::error(500, error.to_string()),
    };
    let _permit = if crate::admission::command_requires_admission(&command) {
        let _placement = state.placement.lock().await;
        match state.admission.enter().await {
            Ok(permit) => Some(permit),
            Err(error) => return RpcReply::error(503, error),
        }
    } else {
        None
    };
    let _brain_control = if state
        .fleet
        .index(id)
        .await
        .ok()
        .flatten()
        .is_some_and(|i| i.kind == ExecutionKind::Brain)
    {
        Some(state.brain_gate.lock(id).await)
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
/// One DAG step's event page; the node picks the step's event source.
pub async fn dag_step_events_id(state: &AppState, id: &str, step: &str, after: i64) -> RpcReply {
    let step = step.to_owned();
    for_id(state, id, move |execution| NodeOperation::DagStepEvents {
        execution,
        step,
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
