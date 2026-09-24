//! Presentation contains references and bounded reasons, never child bodies.
use super::read;
use crate::{api::response, AppState};
use axum::{
    extract::{Path, Query, State},
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

#[derive(serde::Deserialize, Default)]
pub struct VisitQuery {
    pub activation: Option<u64>,
}

pub async fn layer(
    State(state): State<Arc<AppState>>,
    Path((id, layer)): Path<(String, u32)>,
    Query(query): Query<VisitQuery>,
) -> Response {
    response(match detail(&state, &id, layer, query.activation).await {
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
    let mut problem_results = vec![];
    let is_pc = opencoder_core::brain::pc_issue::is_plan(&request.plan);
    let problem = is_pc.then(|| request.inputs.get("problem")).flatten();
    if is_pc {
        for stage in opencoder_core::brain::pc_issue::STAGES {
            if let Some(op) = snapshot
                .operations
                .iter()
                .filter(|op| op.node_id == stage && op.status.successful())
                .max_by_key(|op| op.activation)
            {
                let index = state
                    .fleet
                    .index(&op.execution_id)
                    .await
                    .map_err(read::internal)?
                    .ok_or_else(|| read::internal("PC evidence execution missing"))?;
                let result = read::output(state, &index, "")
                    .await
                    .map_err(read::internal)?;
                problem_results.push(json!({"stage":stage,"execution_id":op.execution_id,"round":op.round,"result":result}));
            }
        }
    }
    Ok(
        json!({"schema_version":request.schema_version,"run":run,"plan":request.plan,
        "layers":layers,"operations":snapshot.operations,"events":events,"capabilities":capabilities,"problem":problem,"problem_results":problem_results}),
    )
}

async fn detail(
    state: &Arc<AppState>,
    id: &str,
    layer: u32,
    activation: Option<u64>,
) -> Result<Value, RpcReply> {
    let (_, request, snapshot) = open(state, id).await?;
    let layers = plan_layers(&request.plan)?;
    let node_ids = layer
        .checked_sub(1)
        .and_then(|index| layers.get(index as usize))
        .ok_or_else(|| RpcReply::error(404, "layered layer not found"))?;
    let events = read::events(state, id, snapshot.run.last_event_seq).await?;
    if request.schema_version >= 5 {
        let visits: Vec<_> = events
            .iter()
            .filter(|e| e.layer == layer && e.event_type == "layer_started")
            .collect();
        if visits.is_empty()
            && (layer <= snapshot.run.layer
                || snapshot.operations.iter().any(|op| op.layer == layer))
        {
            return Err(RpcReply::error(
                500,
                format!("layer {layer} dispatch decision is missing from the event journal"),
            ));
        }
        let selected = if let Some(activation) = activation {
            Some(
                *visits
                    .iter()
                    .find(|e| e.activation == activation)
                    .ok_or_else(|| RpcReply::error(404, "layer activation not found"))?,
            )
        } else {
            visits.last().copied()
        };
        let operations: Vec<_> = snapshot
            .operations
            .iter()
            .filter(|op| selected.is_some_and(|visit| op.activation == visit.activation))
            .collect();
        let assessments = selected.and_then(|visit| {
            events
                .iter()
                .rev()
                .find(|e| e.activation == visit.activation && e.event_type == "milestones_assessed")
        });
        let nodes: Vec<_> = node_ids
            .iter()
            .map(|id| {
                json!({"node_id":id,"milestone":request.plan.node(id),
            "operations":operations.iter().filter(|op| &op.node_id == id).collect::<Vec<_>>(),
            "assessment":assessments.and_then(|e| e.assessments.get(id))})
            })
            .collect();
        return Ok(
            json!({"schema_version":request.schema_version,"layer":layer,"run_phase":snapshot.run.phase,
            "visit":selected,"visits":visits,"nodes":nodes,
            "milestone":request.plan.layers.get(layer as usize - 1),
            "assessment":request.plan.layers.get(layer as usize - 1).and_then(|milestone| assessments.and_then(|event| event.assessments.get(&milestone.layer_id)))}),
        );
    }
    let decision = layer_dispatch(&events, layer, snapshot.run.layer)?;
    let mut nodes = vec![];
    for node_id in node_ids {
        nodes.push(node_row(&request, &snapshot, node_id));
    }
    Ok(
        json!({"schema_version":request.schema_version,"layer":layer,
        "phase":if decision.is_some() { LayeredPhase::Waiting } else { snapshot.run.phase },
        "decision":decision.map(|_| "dispatch_layer"),
        "reason":decision.and_then(|event| event.reason_summary.clone()).unwrap_or_default(),
        "evidence_execution_ids":decision.map(|event| event.evidence_execution_ids.clone()).unwrap_or_default(),
        "nodes":nodes}),
    )
}

/// Child receipts and run completion never replace a layer's dispatch facts.
fn layer_dispatch(
    events: &[LayeredEvent],
    layer: u32,
    dispatched: u32,
) -> Result<Option<&LayeredEvent>, RpcReply> {
    let decision = events.iter().find(|event| {
        event.layer == layer
            && event.event_type == "layer_started"
            && event.decision_summary.as_deref() == Some("dispatch_layer")
    });
    if decision.is_none() && layer <= dispatched {
        return Err(RpcReply::error(
            500,
            format!("layer {layer} dispatch decision is missing from the event journal"),
        ));
    }
    Ok(decision)
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
        "attempts":plan.map(|node| node.retry.as_ref().map(|r| r.max_attempts).unwrap_or(2)).unwrap_or_default(),
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
        || !matches!(
            assignment.request.input["schema_version"].as_u64(),
            Some(4..=7)
        )
    {
        return Err(RpcReply::error(404, "layered run not found"));
    }
    let request: LayeredRequest =
        serde_json::from_value(assignment.request.input["layered_request"].clone())
            .map_err(read::internal)?;
    let snapshot = read::snapshot(state, id).await?;
    Ok((assignment, request, snapshot))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(layer: u32, kind: &str, decision: &str, reason: &str) -> LayeredEvent {
        serde_json::from_value(json!({
            "seq":1,"run_id":"brain-projection","layer":layer,"event_type":kind,
            "decision_summary":decision,"reason_summary":reason,
            "evidence_execution_ids":["agent-upstream"],"at_ms":1
        }))
        .unwrap()
    }

    #[test]
    fn dispatch_survives_child_receipts_retry_failure_and_closing() {
        let dispatch = event(
            1,
            "layer_started",
            "dispatch_layer",
            "parallel evidence collection",
        );
        let mut events = vec![dispatch.clone()];
        for (kind, summary) in [
            ("operation_terminal", "done"),
            ("operation_terminal", "error"),
            ("operation_retry_scheduled", "dispatch_layer"),
            ("run_failed", "error"),
            ("run_completed", "complete"),
        ] {
            events.push(event(1, kind, summary, "later unrelated reason"));
            let found = layer_dispatch(&events, 1, 1).unwrap().unwrap();
            assert_eq!(found, &dispatch);
            assert_eq!(found.evidence_execution_ids, ["agent-upstream"]);
        }
        events.push(event(2, "layer_started", "dispatch_layer", "next layer"));
        assert_eq!(layer_dispatch(&events, 1, 2).unwrap(), Some(&dispatch));
    }

    #[test]
    fn missing_dispatch_is_an_error_only_for_an_already_dispatched_layer() {
        let events = vec![event(1, "operation_terminal", "done", "")];
        assert!(layer_dispatch(&events, 1, 1).is_err());
        assert!(layer_dispatch(&events, 2, 1).unwrap().is_none());
        assert!(layer_dispatch(&[], 1, 0).unwrap().is_none());
    }
}
