use super::{error_400, error_500, response};
use crate::AppState;
use axum::{
    extract::{Path, State},
    response::Response,
    Json,
};
use opencoder_core::fleet::*;
use serde_json::{json, Value};
use std::sync::Arc;

pub async fn nodes(State(state): State<Arc<AppState>>) -> Response {
    let nodes: Vec<_> = state
        .hub
        .views()
        .await
        .into_iter()
        .map(|node| {
            let status = if !node.online {
                "lost"
            } else if node
                .snapshot
                .as_ref()
                .is_some_and(|s| s.active_agent_loops > 0)
            {
                "busy"
            } else {
                "idle"
            };
            let mut value = serde_json::to_value(node).expect("finite node snapshot");
            value["status"] = json!(status);
            value
        })
        .collect();
    response(RpcReply::ok(json!({"nodes":nodes})))
}
pub async fn maintain(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(command): Json<ExecutionCommand>,
) -> Response {
    let _permit = if crate::admission::maintenance_requires_admission(&command) {
        let _placement = state.placement.lock().await;
        match state.admission.enter().await {
            Ok(permit) => Some(permit),
            Err(error) => return response(RpcReply::error(503, error)),
        }
    } else {
        None
    };
    response(
        state
            .hub
            .call(&id, NodeOperation::Maintenance { command })
            .await,
    )
}
pub async fn teams(State(state): State<Arc<AppState>>) -> Response {
    match state.fleet.definitions("team").await {
        Ok(teams) => response(RpcReply::ok(json!({
            "teams": teams
                .into_iter()
                .filter(|team| team["name"].as_str() != Some("system"))
                .collect::<Vec<_>>()
        }))),
        Err(error) => error_500(error.to_string()),
    }
}
pub async fn save_team(
    State(state): State<Arc<AppState>>,
    Json(team): Json<TeamDefinition>,
) -> Response {
    if let Err(error) = team.validate() {
        return error_400(error);
    }
    match state
        .fleet
        .put_definition("team", &team.name, &serde_json::to_value(&team).unwrap())
        .await
    {
        Ok(()) => response(RpcReply::ok(json!(team))),
        Err(error) => error_500(error.to_string()),
    }
}
pub async fn dag_defs(State(state): State<Arc<AppState>>) -> Response {
    match state.fleet.definitions("dag").await {
        Ok(defs) => response(RpcReply::ok(json!(defs))),
        Err(error) => error_500(error.to_string()),
    }
}
pub async fn save_dag(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> Response {
    let spec: opencoder_dag::DagSpec =
        match serde_json::from_value(body.get("spec").cloned().unwrap_or(body)) {
            Ok(spec) => spec,
            Err(error) => return error_400(error.to_string()),
        };
    if let Err(errors) = opencoder_dag::validate(&spec) {
        return error_400(errors.join("; "));
    }
    let definition = json!({"id":spec.name,"name":spec.name,"spec":spec});
    match state
        .fleet
        .put_definition("dag", &spec.name, &definition)
        .await
    {
        Ok(()) => response(RpcReply::ok(definition)),
        Err(error) => error_500(error.to_string()),
    }
}

/// Resolve immutable global definitions before placement. Node-side validation
/// checks installed resources and execution credentials before acceptance.
pub async fn resolve(
    state: &AppState,
    request: &CreateExecution,
) -> Result<Option<Value>, RpcReply> {
    let fail = |e: anyhow::Error| RpcReply::error(500, format!("definition: {e:#}"));
    let definition = match request.kind {
        ExecutionKind::Team | ExecutionKind::Dag => {
            if request.kind == ExecutionKind::Team && request.target.as_deref() == Some("system") {
                return Err(RpcReply::error(400, "system team execution is retired"));
            }
            if let Some(value) = request.input.get("definition") {
                Some(value.clone())
            } else {
                let target = request
                    .target
                    .as_deref()
                    .ok_or_else(|| RpcReply::error(400, "target required"))?;
                Some(
                    state
                        .fleet
                        .definition(request.kind.prefix(), target)
                        .await
                        .map_err(fail)?
                        .ok_or_else(|| RpcReply::error(404, "definition not found"))?,
                )
            }
        }
        ExecutionKind::System => {
            return Err(RpcReply::error(
                400,
                "system team execution is retired; use explicit node maintenance",
            ))
        }
        ExecutionKind::Todos => {
            if let Some(spec) = request.input.get("spec") {
                Some(spec.clone())
            } else {
                let target = request
                    .target
                    .as_deref()
                    .ok_or_else(|| RpcReply::error(400, "template/version target required"))?;
                let (name, version) = target
                    .split_once('/')
                    .ok_or_else(|| RpcReply::error(400, "target must be template/version"))?;
                opencoder_core::validate_share_name(name).map_err(|e| RpcReply::error(400, e))?;
                opencoder_core::validate_share_name(version)
                    .map_err(|e| RpcReply::error(400, e))?;
                let (_, root) = crate::api_todo_util::share_root(&state.workdir)
                    .await
                    .map_err(fail)?;
                Some(
                    super::template::snapshot(&root, name, version)
                        .map_err(|e| RpcReply::error(400, format!("template: {e:#}")))?,
                )
            }
        }
        ExecutionKind::Project => {
            let todo_id = request
                .target
                .as_deref()
                .ok_or_else(|| RpcReply::error(400, "project todo target required"))?;
            let todo = state
                .projects
                .get_todo(todo_id)
                .await
                .map_err(fail)?
                .ok_or_else(|| RpcReply::error(404, "todo not found"))?;
            Some(
                json!({"todo":todo,"goals":state.projects.list_goals().await.map_err(fail)?,"milestones":state.projects.list_milestones(None).await.map_err(fail)?}),
            )
        }
        ExecutionKind::Agent | ExecutionKind::Maintenance => None,
    };
    if let Some(value) = &definition {
        match request.kind {
            ExecutionKind::Team => serde_json::from_value::<TeamDefinition>(value.clone())
                .map_err(|e| RpcReply::error(400, e.to_string()))?
                .validate()
                .map_err(|e| RpcReply::error(400, e))?,
            ExecutionKind::Dag => {
                let spec =
                    serde_json::from_value(value.get("spec").cloned().unwrap_or(value.clone()))
                        .map_err(|e| RpcReply::error(400, format!("DAG: {e}")))?;
                opencoder_dag::validate(&spec).map_err(|e| RpcReply::error(400, e.join("; ")))?;
            }
            ExecutionKind::Todos => {
                let spec = serde_json::from_value(value.clone())
                    .map_err(|e| RpcReply::error(400, format!("workflow: {e}")))?;
                opencoder_todos::domain::validate_spec(&spec)
                    .map_err(|e| RpcReply::error(400, e.to_string()))?;
            }
            _ => {}
        }
    }
    Ok(definition)
}
