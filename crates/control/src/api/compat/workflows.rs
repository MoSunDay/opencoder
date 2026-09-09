use crate::{
    api::{executions, response},
    AppState,
};
use axum::{
    extract::{Path, State},
    response::Response,
    Json,
};
use futures::{stream, StreamExt};
use opencoder_core::fleet::*;
use serde_json::{json, Value};
use std::sync::Arc;

pub async fn dag_definition(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    match state.fleet.definition("dag", &id).await {
        Ok(Some(value)) => response(RpcReply::ok(value)),
        Ok(None) => response(RpcReply::error(404, "DAG not found")),
        Err(e) => response(RpcReply::error(500, e.to_string())),
    }
}
pub async fn delete_dag(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.fleet.delete_definition("dag", &id).await {
        Ok(()) => response(RpcReply::ok(json!({"ok":true}))),
        Err(e) => response(RpcReply::error(500, e.to_string())),
    }
}
async fn dispatch(
    state: Arc<AppState>,
    kind: ExecutionKind,
    target: String,
    body: Value,
    key: &str,
) -> Response {
    let id = body["id"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{}-{}", kind.prefix(), ulid::Ulid::new()));
    let reply = executions::submit(
        &state,
        CreateExecution {
            id: id.clone(),
            kind,
            target: Some(target),
            input: json!({}),
            node_id: body["node_id"].as_str().map(str::to_owned),
        },
    )
    .await;
    if reply.status != 202 {
        return response(reply);
    }
    response(RpcReply {
        status: 202,
        body: json!({key:id,"execution":reply.body}),
    })
}
pub async fn dispatch_dag(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    dispatch(state, ExecutionKind::Dag, id, body, "run_id").await
}
pub async fn dispatch_todos(
    State(state): State<Arc<AppState>>,
    Path((name, version)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    dispatch(
        state,
        ExecutionKind::Todos,
        format!("{name}/{version}"),
        body,
        "workflow_id",
    )
    .await
}
fn dag_view(detail: Value) -> Value {
    let mut value = detail["execution"].clone();
    value["dag_id"] = detail["request"]["target"].clone();
    value["spec"] = detail["definition"]
        .get("spec")
        .unwrap_or(&detail["definition"])
        .clone();
    value["error"] = detail["error"].clone();
    value
}
async fn inspect(state: &AppState, id: &str) -> RpcReply {
    executions::inspect_id(state, id).await
}
pub async fn dag(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let reply = inspect(&state, &id).await;
    if reply.status != 200 {
        return response(reply);
    }
    response(RpcReply::ok(dag_view(reply.body)))
}
pub async fn dag_progress(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    response(
        executions::for_id(&state, &id, |execution| NodeOperation::DagSteps {
            execution,
            step: None,
        })
        .await,
    )
}
pub async fn dag_step(
    State(state): State<Arc<AppState>>,
    Path((id, step)): Path<(String, String)>,
) -> Response {
    response(
        executions::for_id(&state, &id, |execution| NodeOperation::DagSteps {
            execution,
            step: Some(step),
        })
        .await,
    )
}
pub async fn todo(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let reply = inspect(&state, &id).await;
    if reply.status != 200 {
        return response(reply);
    }
    response(RpcReply::ok(reply.body["workflow"].clone()))
}
async fn list(state: Arc<AppState>, kind: ExecutionKind) -> Response {
    let indexes = match state.fleet.indexes(None, Some(kind), 200).await {
        Ok(rows) => rows,
        Err(e) => return response(RpcReply::error(500, e.to_string())),
    };
    let rows: Vec<_> = stream::iter(indexes)
        .map(|index| {
            let state = state.clone();
            async move {
                let reply = inspect(&state, &index.id).await;
                if reply.status != 200 {
                    let mut value = json!(index);
                    value["execution_status"] = value["status"].clone();
                    value["execution_created_at"] = value["created_at"].clone();
                    value["detail_error"] = reply.body;
                    return value;
                }
                if kind == ExecutionKind::Dag {
                    dag_view(reply.body)
                } else {
                    let mut value = reply.body["workflow"]["workflow"].clone();
                    value["node_id"] = json!(index.node_id);
                    value["execution_status"] = json!(index.status);
                    value["execution_created_at"] = json!(index.created_at);
                    value
                }
            }
        })
        .buffered(8)
        .collect()
        .await;
    response(RpcReply::ok(if kind == ExecutionKind::Dag {
        json!(rows)
    } else {
        json!({"workflows":rows})
    }))
}
pub async fn dags(State(state): State<Arc<AppState>>) -> Response {
    list(state, ExecutionKind::Dag).await
}
pub async fn todos(State(state): State<Arc<AppState>>) -> Response {
    list(state, ExecutionKind::Todos).await
}
async fn control(state: Arc<AppState>, id: String, action: &str) -> Response {
    response(
        executions::command_id(
            &state,
            &id,
            ExecutionCommand {
                action: action.into(),
                input: Value::Null,
            },
        )
        .await,
    )
}
pub async fn cancel(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    control(state, id, "cancel").await
}
pub async fn interrupt(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    control(state, id, "interrupt").await
}
pub async fn resume(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    control(state, id, "resume").await
}
