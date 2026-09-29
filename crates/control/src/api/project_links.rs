use crate::{api::response, AppState};
use axum::{
    extract::{Path, State},
    response::Response,
    Json,
};
use opencoder_core::fleet::{ExecutionKind, RpcReply};
use opencoder_store::project::ProjectAssignment;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct LinkBody {
    pub execution_id: String,
}

fn linkable(kind: ExecutionKind) -> bool {
    matches!(
        kind,
        ExecutionKind::Brain
            | ExecutionKind::Dag
            | ExecutionKind::Todos
            | ExecutionKind::Team
            | ExecutionKind::Agent
            | ExecutionKind::Operator
    )
}

pub async fn list(State(state): State<Arc<AppState>>, Path(todo_id): Path<String>) -> Response {
    match state.projects.get_todo(&todo_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return response(RpcReply::error(404, "todo not found")),
        Err(error) => return response(RpcReply::error(500, error.to_string())),
    }
    match state.projects.list_todo_assignments(&todo_id).await {
        Ok(assignments) => {
            let execution_ids: Vec<&str> = assignments
                .iter()
                .map(|item| item.execution_id.as_str())
                .collect();
            response(RpcReply::ok(
                json!({ "assignments": assignments, "execution_ids": execution_ids }),
            ))
        }
        Err(error) => response(RpcReply::error(500, error.to_string())),
    }
}

pub async fn link(
    State(state): State<Arc<AppState>>,
    Path(todo_id): Path<String>,
    Json(body): Json<LinkBody>,
) -> Response {
    match state.projects.get_todo(&todo_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return response(RpcReply::error(404, "todo not found")),
        Err(error) => return response(RpcReply::error(500, error.to_string())),
    }
    let execution_id = body.execution_id.trim();
    let index = match state.fleet.index(execution_id).await {
        Ok(Some(index)) if linkable(index.kind) => index,
        Ok(Some(_)) => return response(RpcReply::error(400, "unsupported execution kind")),
        Ok(None) => return response(RpcReply::error(404, "execution not found")),
        Err(error) => return response(RpcReply::error(500, error.to_string())),
    };
    let names = match state
        .fleet
        .execution_names(std::slice::from_ref(&index.id))
        .await
    {
        Ok(names) => names,
        Err(error) => return response(RpcReply::error(500, error.to_string())),
    };
    let mut name = names
        .get(&index.id)
        .and_then(|names| super::executions::paging::display_name(index.kind, names))
        .unwrap_or_default();
    if name.is_empty() && index.kind == ExecutionKind::Brain {
        name = match state.fleet.assignment(&index.id).await {
            Ok(Some(value)) => value.request.input["layered_request"]["plan"]["title"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            Ok(None) => String::new(),
            Err(error) => return response(RpcReply::error(500, error.to_string())),
        };
    }
    if name.is_empty() {
        name = index.id.clone();
    }
    name = name.chars().take(255).collect();
    let assignment = ProjectAssignment {
        todo_id,
        execution_id: index.id.clone(),
        kind: index.kind.prefix().into(),
        name,
        created_at: opencoder_core::message::now_ms(),
        result_md: None,
        sync_state: "pending".into(),
    };
    match state.projects.link_todo_execution(&assignment).await {
        Ok(()) => response(RpcReply::ok(json!({ "execution_id": index.id }))),
        Err(error) => response(RpcReply::error(500, error.to_string())),
    }
}

pub async fn unlink(
    State(state): State<Arc<AppState>>,
    Path((todo_id, execution_id)): Path<(String, String)>,
) -> Response {
    match state
        .projects
        .unlink_todo_execution(&todo_id, &execution_id)
        .await
    {
        Ok(true) => response(RpcReply::ok(json!({ "deleted": true }))),
        Ok(false) => response(RpcReply::error(404, "execution link not found")),
        Err(error) => response(RpcReply::error(500, error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_agent_execution_capabilities_can_be_linked() {
        for kind in [
            ExecutionKind::Brain,
            ExecutionKind::Dag,
            ExecutionKind::Todos,
            ExecutionKind::Team,
            ExecutionKind::Agent,
            ExecutionKind::Operator,
        ] {
            assert!(linkable(kind));
        }
        for kind in [
            ExecutionKind::Project,
            ExecutionKind::Maintenance,
            ExecutionKind::System,
        ] {
            assert!(!linkable(kind));
        }
    }
}
