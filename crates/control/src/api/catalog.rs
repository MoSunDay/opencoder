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
/// Negotiates node task-file support and the executing node image digest.
pub async fn execution_capabilities(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    if !valid_id(&id) {
        return error_400("invalid node id".into());
    }
    response(
        state
            .hub
            .call(
                &id,
                NodeOperation::Brain {
                    execution: ExecutionRef {
                        id: "dag-capability-probe".into(),
                        kind: ExecutionKind::Dag,
                    },
                    action: "capability_probe".into(),
                    input: json!({"private_files":true}),
                },
            )
            .await,
    )
}

pub async fn unregister(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    use crate::transport::UnregisterResult;
    match state.hub.unregister(&id, &state.fleet).await {
        Ok(UnregisterResult::Removed) => response(RpcReply::ok(json!({"ok": true}))),
        Ok(UnregisterResult::NotFound) => response(RpcReply::error(404, "node not found")),
        Ok(UnregisterResult::Connected) => response(RpcReply::error(
            409,
            "节点仍有活动连接，请先停止节点服务，离线后再删除注册",
        )),
        Err(error) => error_500(format!("unregister node: {error:#}")),
    }
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
    Json(mut team): Json<TeamDefinition>,
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
        match opencoder_dag::decode_spec(body.get("spec").unwrap_or(&body)) {
            Ok(spec) => spec,
            Err(error) => return error_400(error),
        };
    if let Err(errors) = opencoder_dag::validate(&spec) {
        return error_400(errors.join("; "));
    }
    // A rolling release may have multiple Server processes saving the same
    // definition. Serialize the read/replace so creation time stays stable.
    let _lock = match state.fleet.request_lock("dag_definition", &spec.name).await {
        Ok(lock) => lock,
        Err(error) => return error_500(error.to_string()),
    };
    let previous = match state.fleet.definition("dag", &spec.name).await {
        Ok(previous) => previous,
        Err(error) => return error_500(error.to_string()),
    };
    let definition = dag_definition(&spec, previous.as_ref(), opencoder_core::message::now_ms());
    match state
        .fleet
        .put_definition("dag", &spec.name, &definition)
        .await
    {
        Ok(()) => response(RpcReply::ok(definition)),
        Err(error) => error_500(error.to_string()),
    }
}

pub(crate) fn dag_definition(
    spec: &opencoder_dag::DagSpec,
    previous: Option<&Value>,
    now: i64,
) -> Value {
    // Legacy definitions have no timestamps; their first save starts tracking
    // them. Client-supplied timestamps never override server-owned metadata.
    let created_at = previous
        .and_then(|value| value["created_at"].as_i64())
        .unwrap_or(now);
    let updated_at = previous
        .and_then(|value| value["updated_at"].as_i64())
        .map_or(now, |last| now.max(last.saturating_add(1)));
    json!({
        "id": spec.name, "name": spec.name, "spec": spec,
        "created_at": created_at, "updated_at": updated_at,
    })
}

/// Resolve immutable global definitions before placement. Node-side validation
/// checks installed resources and execution credentials before acceptance.
pub async fn resolve(
    state: &AppState,
    request: &CreateExecution,
) -> Result<Option<Value>, RpcReply> {
    let fail = |e: anyhow::Error| RpcReply::error(500, format!("definition: {e:#}"));
    let mut definition = match request.kind {
        ExecutionKind::Brain => {
            if request.input["schema_version"]
                == opencoder_core::brain::layered::LAYERED_SCHEMA_VERSION
            {
                let layered: opencoder_core::brain::layered::LayeredRequest =
                    serde_json::from_value(request.input["layered_request"].clone())
                        .map_err(|e| RpcReply::error(400, format!("layered request: {e}")))?;
                opencoder_brain::layered::validate_request(&layered)
                    .map_err(|e| RpcReply::error(400, e.to_string()))?;
                return Ok(Some(request.input.clone()));
            }
            return Err(RpcReply::error(409, "unsupported brain schema; expected 4"));
        }
        ExecutionKind::Team | ExecutionKind::Dag => {
            if request.kind == ExecutionKind::Dag
                && request.target.as_deref() == Some(opencoder_dag::devices::NAME)
            {
                let spec = opencoder_dag::devices::definition(&request.input)
                    .map_err(|e| RpcReply::error(400, e))?;
                return Ok(Some(
                    serde_json::to_value(spec).map_err(|e| fail(e.into()))?,
                ));
            }
            if request.kind == ExecutionKind::Dag
                && request.target.as_deref() == Some(opencoder_dag::ui_cases::NAME)
            {
                let spec = opencoder_dag::ui_cases::definition(&request.input)
                    .map_err(|e| RpcReply::error(400, e))?;
                return Ok(Some(
                    serde_json::to_value(spec).map_err(|e| fail(e.into()))?,
                ));
            }
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
        ExecutionKind::Agent | ExecutionKind::Maintenance | ExecutionKind::Operator => None,
    };
    if let Some(value) = definition.as_mut() {
        match request.kind {
            ExecutionKind::Team => {
                let mut team = serde_json::from_value::<TeamDefinition>(value.clone())
                    .map_err(|e| RpcReply::error(400, e.to_string()))?;
                team.validate().map_err(|e| RpcReply::error(400, e))?;
                // Capabilities are control-plane state, not user input: the
                // pinned definition freezes each member agent's bound
                // capability summaries (empty when the agent has none).
                let groups = super::brain::agent_capability_groups(state)
                    .await
                    .map_err(fail)?;
                for member in &mut team.members {
                    member.capabilities = groups
                        .iter()
                        .find(|(agent, _)| *agent == member.agent)
                        .map(|(_, capabilities)| {
                            capabilities
                                .iter()
                                .filter_map(|c| c["summary"].as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default();
                }
                *value = serde_json::to_value(&team)
                    .map_err(|e| RpcReply::error(500, format!("definition: {e}")))?;
            }
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
