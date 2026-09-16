use super::{error_400, error_500, response};
use crate::AppState;
use axum::{
    extract::{Path, Query, State},
    response::Response,
    Json,
};
use futures::{stream, StreamExt};
use opencoder_core::fleet::*;
use opencoder_store::ProjectExecutorKind;
use serde_json::{json, Value};
use std::sync::Arc;
mod routing;
pub(super) use routing::{brain_preresolve, initial_receipt};

pub async fn overview(State(state): State<Arc<AppState>>) -> Response {
    let result = async {
        let goals = state.projects.list_goals().await?;
        let milestones = state.projects.list_milestones(None).await?;
        let todos = state.projects.list_todos(None).await?;
        let items: Vec<Value> = stream::iter(todos)
            .map(|todo| {
                let state = state.clone();
                async move {
                    let mut value = json!(todo);
                    let id = format!("project-{}", todo.id);
                    if let Some(index) = state.fleet.index(&id).await? {
                        value["execution"] = json!(index);
                        let reply = state
                            .hub
                            .call(
                                &index.node_id,
                                NodeOperation::Inspect {
                                    execution: index.execution_ref(),
                                },
                            )
                            .await;
                        if reply.status == 200 {
                            for key in ["status", "plan_md", "active_session_id"] {
                                value[key] = reply.body["todo"][key].clone();
                            }
                        } else {
                            value["detail_error"] = reply.body;
                        }
                    }
                    Ok::<_, anyhow::Error>(value)
                }
            })
            .buffered(8)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<anyhow::Result<_>>()?;
        Ok::<_, anyhow::Error>(opencoder_store::project::overview::overview(
            &goals,
            &milestones,
            &items,
        ))
    }
    .await;
    match result {
        Ok(value) => response(RpcReply::ok(value)),
        Err(error) => error_500(error.to_string()),
    }
}
pub async fn runs(
    State(state): State<Arc<AppState>>,
    Path(todo): Path<String>,
    Query(query): Query<super::executions::ProjectRunsQuery>,
) -> Response {
    if query.before_version.is_some_and(|version| version <= 0) {
        return error_400("invalid project run cursor".into());
    }
    let id = format!("project-{todo}");
    match state.fleet.index(&id).await {
        Ok(None) => response(RpcReply::ok(
            json!({"runs":[],"next_version":null,"more":false}),
        )),
        Ok(Some(_)) => response(
            super::executions::for_id(&state, &id, |execution| NodeOperation::ProjectRuns {
                execution,
                before_version: query.before_version,
            })
            .await,
        ),
        Err(error) => error_500(error.to_string()),
    }
}
pub async fn plan(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Option<Json<Value>>,
) -> Response {
    start(state, id, "plan", body.map(|b| b.0).unwrap_or(json!({}))).await
}
pub async fn execute(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Option<Json<Value>>,
) -> Response {
    start(state, id, "execute", body.map(|b| b.0).unwrap_or(json!({}))).await
}
async fn start(state: Arc<AppState>, todo: String, action: &str, mut input: Value) -> Response {
    if !input.is_object() {
        return error_400("project input must be an object".into());
    }
    if input.get("run_id").is_some_and(|id| {
        !id.as_str()
            .is_some_and(|id| id.starts_with("prun-") && valid_id(id))
    }) {
        return error_400("invalid project run id".into());
    }
    if input.get("run_id").is_none() {
        input["run_id"] = json!(format!("prun-{}", ulid::Ulid::new()));
    }
    match initial_receipt(&state, &todo, action, &input).await {
        Ok(Some(reply)) | Err(reply) => return response(reply),
        Ok(None) => {}
    }
    let id = format!("project-{todo}");
    match state.fleet.index(&id).await {
        Ok(Some(index)) => {
            let reply = super::executions::dispatch_command(
                &state,
                &id,
                ExecutionCommand {
                    action: action.into(),
                    input: input.clone(),
                },
            )
            .await;
            if reply.status == 404
                && reply.body["error"] == "execution not found"
                && matches!(
                    index.status,
                    ExecutionStatus::Pending | ExecutionStatus::Error
                )
            {
                submit_start(state, todo, action, input).await
            } else {
                response(reply)
            }
        }
        Ok(None) => submit_start(state, todo, action, input).await,
        Err(e) => error_500(e.to_string()),
    }
}
async fn submit_start(state: Arc<AppState>, todo: String, action: &str, input: Value) -> Response {
    let id = format!("project-{todo}");

    let input = match brain_preresolve(&state, &todo, action, input).await {
        Ok(input) => input,
        Err(reply) => return response(reply),
    };
    let mut request_input = input.clone();
    request_input.as_object_mut().map(|o| o.remove("node_id"));
    request_input["action"] = json!(action);
    response(
        super::executions::submit(
            &state,
            CreateExecution {
                id,
                kind: ExecutionKind::Project,
                target: Some(todo),
                node_id: input["node_id"].as_str().map(str::to_owned),
                input: request_input,
            },
        )
        .await,
    )
}

/// Preserve historical Project records while rejecting retired execution paths.
async fn resolve_brain_executor(
    state: &Arc<AppState>,
    todo: &str,
    action: &str,
    input: Value,
) -> Result<Value, RpcReply> {
    if input.get("brain").is_some_and(|v| !v.is_null()) {
        return Err(RpcReply::error(409, opencoder_brain::graph::MIGRATION));
    }
    if action == "execute" {
        if let Some(record) = state
            .projects
            .get_todo(todo)
            .await
            .map_err(|e| RpcReply::error(500, e.to_string()))?
        {
            if matches!(
                record.executor_kind,
                ProjectExecutorKind::Brain | ProjectExecutorKind::Playbook
            ) {
                return Err(RpcReply::error(409, opencoder_brain::graph::MIGRATION));
            }
        }
    }
    Ok(input)
}
