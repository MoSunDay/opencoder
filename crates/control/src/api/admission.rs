use super::response;
use crate::{admission::AdmissionMode, AppState};
use axum::{
    extract::{Path, Query, State},
    response::Response,
};
use futures::future::join_all;
use opencoder_core::fleet::{NodeAdmissionCommand, NodeOperation, RpcReply};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

async fn node_command(state: &Arc<AppState>, id: &str, command: NodeAdmissionCommand) -> Response {
    if command == NodeAdmissionCommand::Status {
        return response(
            state
                .hub
                .call(id, NodeOperation::Admission { command })
                .await,
        );
    }
    let _process_lock = match state.fleet.request_lock("admission", "cluster").await {
        Ok(lock) => lock,
        Err(error) => return response(RpcReply::error(500, error.to_string())),
    };
    let _transition = state.admission.transition().await;
    if command == NodeAdmissionCommand::Reopen && !state.admission.is_open().await {
        return response(RpcReply::error(409, "Server admission is frozen"));
    }
    let reply = state
        .hub
        .call(id, NodeOperation::Admission { command })
        .await;
    response(reply)
}

pub async fn node_status(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    node_command(&state, &id, NodeAdmissionCommand::Status).await
}

pub async fn node_freeze(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    node_command(&state, &id, NodeAdmissionCommand::Freeze).await
}

pub async fn node_reopen(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    node_command(&state, &id, NodeAdmissionCommand::Reopen).await
}

#[derive(Debug, Serialize)]
struct NodeAdmissionResult {
    node_id: String,
    status: u16,
    body: serde_json::Value,
}

async fn call_online_nodes(
    state: &Arc<AppState>,
    command: NodeAdmissionCommand,
    selected: Option<&str>,
) -> (Vec<NodeAdmissionResult>, Vec<String>) {
    let views: Vec<_> = state
        .hub
        .views()
        .await
        .into_iter()
        .filter(|node| selected.is_none_or(|id| node.registration.id == id))
        .collect();
    let offline = views
        .iter()
        .filter(|node| !node.online)
        .map(|node| node.registration.id.clone())
        .collect();
    let calls = views.into_iter().filter(|node| node.online).map(|node| {
        let state = Arc::clone(state);
        async move {
            let node_id = node.registration.id;
            let reply = state
                .hub
                .call(&node_id, NodeOperation::Admission { command })
                .await;
            NodeAdmissionResult {
                node_id,
                status: reply.status,
                body: reply.body,
            }
        }
    });
    (join_all(calls).await, offline)
}

async fn local_status(state: &Arc<AppState>) -> anyhow::Result<serde_json::Value> {
    let admission = state.admission.snapshot().await?;
    let views = state.hub.views().await;
    let online_nodes = views.iter().filter(|node| node.online).count();
    let ready_nodes = views
        .iter()
        .filter(|node| {
            node.online
                && node
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.ready)
        })
        .count();
    let active_executions = state.fleet.active_execution_count().await?;
    Ok(json!({
        "mode": admission.mode,
        "inflight_admissions": admission.inflight_admissions,
        "active_executions": active_executions,
        "online_nodes": online_nodes,
        "ready_nodes": ready_nodes,
        "control_drained": admission.mode == AdmissionMode::Frozen
            && admission.inflight_admissions == 0
            && active_executions == 0,
    }))
}

async fn ready_status(state: &Arc<AppState>) -> anyhow::Result<serde_json::Value> {
    if let Some(ontology) = &state.ontology {
        ontology.ready().await?;
    }
    let admission = state.admission.snapshot().await?;
    if admission.mode != AdmissionMode::Open {
        return local_status(state).await;
    }
    let views = state.hub.views().await;
    Ok(json!({
        "mode": admission.mode,
        "inflight_admissions": admission.inflight_admissions,
        "online_nodes": views.iter().filter(|node| node.online).count(),
        "ready_nodes": views.iter().filter(|node| node.online && node.snapshot.as_ref().is_some_and(|snapshot| snapshot.ready)).count(),
        "control_drained": false,
    }))
}

fn cluster_drained(server: &serde_json::Value, nodes: &[NodeAdmissionResult]) -> bool {
    server["control_drained"] == true
        && nodes.iter().all(|node| {
            (200..300).contains(&node.status)
                && node.body["mode"] == "frozen"
                && node.body["active_runs"].as_u64() == Some(0)
                && node.body["owned_processes"].as_u64() == Some(0)
        })
}

pub async fn ready(State(state): State<Arc<AppState>>) -> Response {
    match ready_status(&state).await {
        Ok(status)
            if status["mode"] == "open"
                && status["ready_nodes"]
                    .as_u64()
                    .is_some_and(|count| count > 0) =>
        {
            response(RpcReply::ok(status))
        }
        Ok(status) => response(RpcReply {
            status: 503,
            body: status,
        }),
        Err(error) => response(RpcReply::error(500, error.to_string())),
    }
}

pub async fn status(State(state): State<Arc<AppState>>) -> Response {
    let status = match local_status(&state).await {
        Ok(status) => status,
        Err(error) => return response(RpcReply::error(500, error.to_string())),
    };
    let (nodes, offline_nodes) =
        call_online_nodes(&state, NodeAdmissionCommand::Status, None).await;
    let drained = cluster_drained(&status, &nodes);
    response(RpcReply::ok(json!({
        "server": status,
        "nodes": nodes,
        "offline_nodes": offline_nodes,
        "drained": drained,
    })))
}

pub async fn freeze(State(state): State<Arc<AppState>>) -> Response {
    let _process_lock = match state.fleet.request_lock("admission", "cluster").await {
        Ok(lock) => lock,
        Err(error) => return response(RpcReply::error(500, error.to_string())),
    };
    let _transition = state.admission.transition().await;
    if let Err(error) = state.admission.freeze(&state.placement).await {
        return response(RpcReply::error(500, error.to_string()));
    }
    let (nodes, offline_nodes) =
        call_online_nodes(&state, NodeAdmissionCommand::Freeze, None).await;
    let status = local_status(&state)
        .await
        .unwrap_or_else(|error| json!({"mode":"frozen","status_error":error.to_string()}));
    let drained = cluster_drained(&status, &nodes);
    response(RpcReply::ok(json!({
        "server": status,
        "nodes": nodes,
        "offline_nodes": offline_nodes,
        "drained": drained,
    })))
}

#[derive(Deserialize)]
pub struct ReopenScope {
    pub node_id: Option<String>,
}

pub async fn reopen(
    State(state): State<Arc<AppState>>,
    Query(scope): Query<ReopenScope>,
) -> Response {
    let _process_lock = match state.fleet.request_lock("admission", "cluster").await {
        Ok(lock) => lock,
        Err(error) => return response(RpcReply::error(500, error.to_string())),
    };
    let _transition = state.admission.transition().await;
    // Reconcile online nodes even when the server is already open: a node
    // may still carry its durable shutdown freeze.
    let server_open = state.admission.is_open().await;
    let (nodes, offline_nodes) = call_online_nodes(
        &state,
        NodeAdmissionCommand::Reopen,
        scope.node_id.as_deref(),
    )
    .await;
    if scope.node_id.is_some() && nodes.is_empty() {
        return response(RpcReply::error(503, "selected node is unavailable"));
    }
    if nodes.is_empty() && !server_open {
        return response(RpcReply::error(
            503,
            "no online node available for admission verification",
        ));
    }
    if nodes
        .iter()
        .any(|node| !(200..300).contains(&node.status) || node.body["mode"] != "open")
    {
        return response(RpcReply {
            status: 503,
            body: json!({
                "error": if server_open {
                    "one or more online nodes rejected admission reopen"
                } else {
                    "one or more online nodes rejected admission reopen; server remains frozen"
                },
                "nodes": nodes,
                "offline_nodes": offline_nodes,
            }),
        });
    }
    if let Err(error) = state.admission.reopen(&state.placement).await {
        return response(RpcReply::error(500, error.to_string()));
    }
    let status = local_status(&state)
        .await
        .unwrap_or_else(|error| json!({"mode":"open","status_error":error.to_string()}));
    response(RpcReply::ok(json!({
        "server": status,
        "nodes": nodes,
        "offline_nodes": offline_nodes,
    })))
}

pub async fn freeze_cluster(state: &Arc<AppState>) -> anyhow::Result<()> {
    let _transition = state.admission.transition().await;
    state.admission.freeze(&state.placement).await?;
    let (nodes, _) = call_online_nodes(state, NodeAdmissionCommand::Freeze, None).await;
    for node in nodes
        .into_iter()
        .filter(|node| !(200..300).contains(&node.status))
    {
        tracing::warn!(
            node_id = %node.node_id,
            status = node.status,
            body = %node.body,
            "node admission freeze was not acknowledged"
        );
    }
    Ok(())
}
