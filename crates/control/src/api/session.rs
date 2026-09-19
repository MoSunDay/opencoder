//! Existing conversation UI uses these adapters; every operation is node-owned.
use super::{error_400, error_404, error_500, response};
use crate::AppState;
use axum::{
    extract::{Query, Request, State},
    response::Response,
    Json,
};
use futures::{stream, StreamExt};
use opencoder_core::fleet::*;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug, Default, serde::Deserialize)]
pub struct SessionKindQuery {
    pub kind: Option<String>,
}

/// The chat page's optional `kind` selector: default stays `operator`,
/// `agent` launches the same session executor without the operator
/// preamble. Every other value is a client error — dag/team/... keep
/// their dedicated routes instead of this session surface.
fn requested_kind(body: &Value) -> Result<ExecutionKind, String> {
    match body["kind"].as_str() {
        None | Some("operator") => Ok(ExecutionKind::Operator),
        Some("agent") => Ok(ExecutionKind::Agent),
        Some(other) => Err(format!(
            "unsupported session kind {other:?}: use operator or agent"
        )),
    }
}

pub(crate) fn requested_query_kind(value: Option<&str>) -> Result<ExecutionKind, String> {
    match value {
        None | Some("operator") => Ok(ExecutionKind::Operator),
        Some("agent") => Ok(ExecutionKind::Agent),
        Some(other) => Err(format!(
            "unsupported session kind {other:?}: use operator or agent"
        )),
    }
}

pub async fn create(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> Response {
    let kind = match requested_kind(&body) {
        Ok(kind) => kind,
        Err(error) => return error_400(error),
    };
    // A caller-supplied id keeps its verbatim prefix; the prefix/kind
    // mismatch is `CreateExecution::validate`'s job (same message as the
    // generic POST /api/executions route).
    let id = body["id"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{}-{}", kind.prefix(), ulid::Ulid::new()));
    let request = CreateExecution {
        id: id.clone(),
        kind,
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

pub async fn list(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SessionKindQuery>,
) -> Response {
    let kind = match requested_query_kind(query.kind.as_deref()) {
        Ok(kind) => kind,
        Err(error) => return error_400(error),
    };
    match summaries(&state, None, kind).await {
        Ok(sessions) => response(RpcReply::ok(json!({"sessions":sessions}))),
        Err(e) => error_500(e.to_string()),
    }
}
pub async fn summaries(
    state: &Arc<AppState>,
    node: Option<&str>,
    kind: ExecutionKind,
) -> anyhow::Result<Vec<Value>> {
    // Operator and Agent are separate conversation lanes. The caller chooses
    // one kind; no other execution family can leak into either lane.
    let indexes = state.fleet.indexes(node, Some(kind), 500).await?;
    Ok(stream::iter(indexes).map(|index| {let state=state.clone();async move {
        let reply=state.hub.call(&index.node_id,NodeOperation::Command{execution:index.execution_ref(),command:ExecutionCommand{action:"summary".into(),input:Value::Null}}).await;
        let mut meta=reply.body.clone();
        if reply.status!=200 || !meta.is_object() {meta=json!({"id":index.id,"created_at":index.created_at,"status":index.status,"detail_error":reply.body});}
        // Keep the kind in the summary envelope so a dialog row always has
        // an unambiguous reference to its control-plane execution detail.
        meta["kind"] = json!(index.kind);
        meta["execution_ref"] = json!({"id":index.id,"kind":index.kind});
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
