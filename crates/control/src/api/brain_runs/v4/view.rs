//! Presentation contains references and bounded reasons, never child bodies.
use super::read;
use crate::{api::response, AppState};
use axum::{
    extract::{Path, State},
    response::Response,
};
use opencoder_core::{brain::layered::*, brain::*, fleet::*};
use serde_json::{json, Value};
use std::sync::Arc;

pub(super) fn capability_metadata(cap: &BrainCapabilityDescriptor) -> Value {
    json!({"capability_id":cap.capability_id,"kind":cap.kind,"target":cap.target,"version":cap.version,"input_desc":cap.input_desc,"output_desc":cap.output_desc})
}

pub async fn view(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    response(match project(&state, &id).await {
        Ok(view) => RpcReply::ok(view),
        Err(reply) => reply,
    })
}

pub async fn layer(
    State(state): State<Arc<AppState>>,
    Path((id, layer)): Path<(String, u32)>,
) -> Response {
    response(match detail(&state, &id, layer).await {
        Ok(detail) => RpcReply::ok(detail),
        Err(reply) => reply,
    })
}

async fn project(state: &Arc<AppState>, id: &str) -> Result<Value, RpcReply> {
    let (assignment, request, snapshot) = open(state, id).await?;
    let layers = plan_layers(&request.plan)?;
    let capabilities = scope(state, &assignment, &request).await?;
    let events = read::events(state, id, snapshot.run.last_event_seq).await?;
    let mut run = serde_json::to_value(&snapshot.run).map_err(read::internal)?;
    run["total_layers"] = json!(layers.len());
    Ok(
        json!({"schema_version":LAYERED_SCHEMA_VERSION,"run":run,"plan":request.plan,
        "layers":layers,"operations":snapshot.operations,"events":events,"capabilities":capabilities}),
    )
}

async fn detail(state: &Arc<AppState>, id: &str, layer: u32) -> Result<Value, RpcReply> {
    let (_, request, snapshot) = open(state, id).await?;
    let layers = plan_layers(&request.plan)?;
    let node_ids = layer
        .checked_sub(1)
        .and_then(|index| layers.get(index as usize))
        .ok_or_else(|| RpcReply::error(404, "layered layer not found"))?;
    let events = read::events(state, id, snapshot.run.last_event_seq).await?;
    let decision = events
        .iter()
        .rfind(|event| event.layer == layer && event.decision_summary.is_some());
    let mut nodes = vec![];
    for node_id in node_ids {
        nodes.push(node_row(&request, &snapshot, node_id));
    }
    Ok(
        json!({"schema_version":LAYERED_SCHEMA_VERSION,"layer":layer,
        "phase":decision.map(decision_phase).unwrap_or(snapshot.run.phase),
        "reason":decision.and_then(|event| event.reason_summary.clone()).unwrap_or_default(),
        "evidence_execution_ids":decision.map(|event| event.evidence_execution_ids.clone()).unwrap_or_default(),
        "nodes":nodes}),
    )
}

/// The phase a decision moved the layer into, per the frozen decision summary.
fn decision_phase(event: &LayeredEvent) -> LayeredPhase {
    match event.decision_summary.as_deref() {
        Some("dispatch_layer") => LayeredPhase::Waiting,
        Some("complete") => LayeredPhase::Completed,
        _ => LayeredPhase::Deciding,
    }
}

fn node_row(request: &LayeredRequest, snapshot: &LayeredSnapshot, node_id: &str) -> Value {
    let plan = request.plan.node(node_id);
    let op = snapshot
        .operations
        .iter()
        .filter(|op| op.node_id == node_id)
        .max_by_key(|op| op.attempt);
    json!({
        "node_id":node_id,
        "title":plan.map(|node| node.title.clone()).unwrap_or_default(),
        "capability_id":op.map(|op| op.capability_id.clone()).unwrap_or_else(|| plan.map(|node| node.capability_id.clone()).unwrap_or_default()),
        "status":match op { Some(op) => json!(op.status), None => json!("pending") },
        "attempt":op.map(|op| op.attempt).unwrap_or(0),
        "attempts":plan.map(|node| node.retry.max_attempts).unwrap_or_default(),
        "execution_id":op.map(|op| op.execution_id.clone()),
        "execution_kind":op.map(|op| op.execution_kind),
        "cancel_requested":op.map(|op| op.cancel_requested).unwrap_or(false),
    })
}

/// The frozen capability scope of the run, or the catalog view of its plan.
async fn scope(
    state: &Arc<AppState>,
    assignment: &Assignment,
    request: &LayeredRequest,
) -> Result<Value, RpcReply> {
    if let Some(scope) = assignment.request.input.get("capability_scope") {
        if scope.is_array() {
            return Ok(scope.clone());
        }
    }
    let capabilities = super::catalog::available(state, request)
        .await
        .map_err(read::internal)?;
    Ok(json!(capabilities
        .iter()
        .map(capability_metadata)
        .collect::<Vec<_>>()))
}

fn plan_layers(plan: &LayeredPlan) -> Result<Vec<Vec<String>>, RpcReply> {
    opencoder_brain::layered::layers(plan).map_err(read::internal)
}

/// A v4 run only: a v3 or v2 run is never served from this route.
async fn open(
    state: &Arc<AppState>,
    id: &str,
) -> Result<(Assignment, LayeredRequest, LayeredSnapshot), RpcReply> {
    let assignment = state
        .fleet
        .assignment(id)
        .await
        .map_err(read::internal)?
        .ok_or_else(|| RpcReply::error(404, "layered run not found"))?;
    if assignment.request.kind != ExecutionKind::Brain
        || assignment.request.input["schema_version"] != LAYERED_SCHEMA_VERSION
    {
        return Err(RpcReply::error(404, "layered run not found"));
    }
    let request: LayeredRequest =
        serde_json::from_value(assignment.request.input["layered_request"].clone())
            .map_err(read::internal)?;
    let snapshot = read::snapshot(state, id).await?;
    Ok((assignment, request, snapshot))
}
