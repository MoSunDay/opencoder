//! Existing conversation UI uses these adapters; every operation is node-owned.
use super::{error_400, error_404, error_500, response};
use crate::AppState;
use axum::{
    extract::{Request, State},
    response::Response,
    Json,
};
use futures::{stream, StreamExt};
use opencoder_core::fleet::*;
use serde_json::{json, Value};
use std::sync::Arc;

pub async fn create(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> Response {
    let id = body["id"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("agent-{}", ulid::Ulid::new()));
    let request = CreateExecution {
        id: id.clone(),
        kind: ExecutionKind::Agent,
        target: body["agent"].as_str().map(str::to_owned),
        input: body.clone(),
        node_id: body["node_id"].as_str().map(str::to_owned),
    };
    let reply = super::executions::submit(&state, request).await;
    if reply.status != 202 {
        return response(reply);
    }
    response(RpcReply::ok(json!({"id":id,"execution":reply.body})))
}

pub async fn list(State(state): State<Arc<AppState>>) -> Response {
    match summaries(&state, None).await {
        Ok(sessions) => response(RpcReply::ok(json!({"sessions":sessions}))),
        Err(e) => error_500(e.to_string()),
    }
}
pub async fn summaries(state: &Arc<AppState>, node: Option<&str>) -> anyhow::Result<Vec<Value>> {
    let indexes = state
        .fleet
        .indexes(node, Some(ExecutionKind::Agent), 500)
        .await?;
    Ok(stream::iter(indexes).map(|index| {let state=state.clone();async move {
        let reply=state.hub.call(&index.node_id,NodeOperation::Command{execution:index.execution_ref(),command:ExecutionCommand{action:"summary".into(),input:Value::Null}}).await;
        let mut meta=reply.body.clone();
        if reply.status!=200 || !meta.is_object() {meta=json!({"id":index.id,"created_at":index.created_at,"status":index.status,"detail_error":reply.body});}
        meta["node_id"]=json!(index.node_id); meta
    }}).buffered(8).collect().await)
}

pub async fn relay(State(state): State<Arc<AppState>>, request: Request) -> Response {
    let method = request.method().to_string();
    let path = request.uri().path().to_string();
    let Some(rest) = path.strip_prefix("/api/sessions/") else {
        return error_404("route not found");
    };
    let (id, tail) = rest.split_once('/').unwrap_or((rest, ""));
    if !valid_id(id) || tail.contains("..") {
        return error_400("invalid session path".into());
    }
    let tail = match request.uri().query() {
        Some(query) => format!("{tail}?{query}"),
        None => tail.into(),
    };
    let bytes = match axum::body::to_bytes(request.into_body(), MAX_FRAME_BYTES).await {
        Ok(bytes) => bytes,
        Err(error) => return error_400(error.to_string()),
    };
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        match serde_json::from_slice(&bytes) {
            Ok(body) => body,
            Err(error) => return error_400(error.to_string()),
        }
    };
    let command = ExecutionCommand {
        action: "http".into(),
        input: json!({"method":method,"tail":tail,"body":body}),
    };
    response(super::executions::command_id(&state, id, command).await)
}
