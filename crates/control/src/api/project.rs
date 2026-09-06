use super::{error_500, response};
use crate::AppState;
use axum::{
    extract::{Path, State},
    response::Response,
    Json,
};
use futures::{stream, StreamExt};
use opencoder_core::fleet::*;
use serde_json::{json, Value};
use std::sync::Arc;

pub async fn overview(State(state): State<Arc<AppState>>) -> Response {
    let result=async {
        let goals=state.projects.list_goals().await?;
        let milestones=state.projects.list_milestones(None).await?;
        let todos=state.projects.list_todos(None).await?;
        let items:Vec<Value>=stream::iter(todos).map(|todo| {let state=state.clone();async move {
            let mut value=json!(todo);let id=format!("project-{}",todo.id);
            if let Some(index)=state.fleet.index(&id).await? {
                value["execution"]=json!(index);
                let reply=state.hub.call(&index.node_id,NodeOperation::Inspect{execution:index.execution_ref()}).await;
                if reply.status==200 {
                    for key in ["status","plan_md","active_session_id"] {value[key]=reply.body["todo"][key].clone();}
                } else {value["detail_error"]=reply.body;}
            }
            Ok::<_,anyhow::Error>(value)
        }}).buffered(8).collect::<Vec<_>>().await.into_iter().collect::<anyhow::Result<_>>()?;
        let nested:Vec<_>=goals.into_iter().map(|goal| {
            let mut value=json!(goal);
            value["milestones"]=json!(milestones.iter().filter(|m|m.goal_id==goal.id).map(|milestone| {
                let mut value=json!(milestone);value["todos"]=json!(items.iter().filter(|t|t["milestone_id"]==milestone.id).collect::<Vec<_>>());value
            }).collect::<Vec<_>>());value
        }).collect();
        Ok::<_,anyhow::Error>(json!({"goals":nested,"backlog":items.iter().filter(|t|t["milestone_id"].is_null()).collect::<Vec<_>>()}))
    }.await;
    match result {
        Ok(value) => response(RpcReply::ok(value)),
        Err(error) => error_500(error.to_string()),
    }
}
pub async fn runs(State(state): State<Arc<AppState>>, Path(todo): Path<String>) -> Response {
    let id = format!("project-{todo}");
    match state.fleet.index(&id).await {
        Ok(None) => response(RpcReply::ok(json!({"runs":[]}))),
        Ok(Some(_)) => {
            let reply = super::executions::inspect_id(&state, &id).await;
            if reply.status != 200 {
                return response(reply);
            }
            response(RpcReply::ok(json!({"runs":reply.body["runs"]})))
        }
        Err(e) => error_500(e.to_string()),
    }
}
pub async fn plan(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Option<Json<Value>>,
) -> Response {
    start(state, id, "plan", body.map(|b| b.0).unwrap_or(json!({}))).await
}
pub async fn execute(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    start(state, id, "execute", json!({})).await
}
async fn start(state: Arc<AppState>, todo: String, action: &str, input: Value) -> Response {
    let id = format!("project-{todo}");
    match state.fleet.index(&id).await {
        Ok(Some(_)) => response(
            super::executions::dispatch_command(
                &state,
                &id,
                ExecutionCommand {
                    action: action.into(),
                    input,
                },
            )
            .await,
        ),
        Ok(None) => response(
            super::executions::submit(
                &state,
                CreateExecution {
                    id,
                    kind: ExecutionKind::Project,
                    target: Some(todo),
                    node_id: input["node_id"].as_str().map(str::to_owned),
                    input: json!({"action":action}),
                },
            )
            .await,
        ),
        Err(e) => error_500(e.to_string()),
    }
}
