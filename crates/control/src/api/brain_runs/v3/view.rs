//! Presentation contains references and bounded reasons, never child bodies.
use crate::{
    api::{brain_runs::runs, response},
    AppState,
};
use axum::{
    extract::{Path, State},
    response::Response,
};
use opencoder_core::{brain::*, fleet::*};
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Arc};

pub(super) fn capability_metadata(cap: &BrainCapabilityDescriptor) -> Value {
    json!({"capability_id":cap.capability_id,"kind":cap.kind,"target":cap.target,"version":cap.version,"input_desc":cap.input_desc,"output_desc":cap.output_desc})
}

pub async fn view(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    response(match project(&state, &id).await {
        Ok(view) => RpcReply::ok(view),
        Err(reply) => reply,
    })
}

pub async fn round(
    State(state): State<Arc<AppState>>,
    Path((id, number)): Path<(String, u32)>,
) -> Response {
    response(match project(&state, &id).await {
        Ok(view) => view["rounds"]
            .as_array()
            .and_then(|rounds| rounds.iter().find(|r| r["round"] == number))
            .cloned()
            .map(RpcReply::ok)
            .unwrap_or_else(|| RpcReply::error(404, "scheduler round not found")),
        Err(reply) => reply,
    })
}

fn internal(error: impl std::fmt::Display) -> RpcReply {
    RpcReply::error(500, error.to_string())
}

async fn project(state: &Arc<AppState>, id: &str) -> Result<Value, RpcReply> {
    let assignment = state
        .fleet
        .assignment(id)
        .await
        .map_err(internal)?
        .ok_or_else(|| RpcReply::error(404, "brain run not found"))?;
    if assignment.request.input["schema_version"] != 3 {
        return Err(RpcReply::error(409, SCHEDULER_MIGRATION));
    }
    let request: BrainSchedulerRequest =
        serde_json::from_value(assignment.request.input["scheduler_request"].clone())
            .map_err(internal)?;
    let reply = runs::call(state, id, "snapshot", Value::Null).await;
    if reply.status >= 300 {
        return Err(reply);
    }
    let snapshot: BrainSchedulerSnapshot = serde_json::from_value(reply.body).map_err(internal)?;
    let capabilities = match assignment.request.input.get("capability_scope") {
        Some(scope) => scope.clone(),
        None => {
            let catalog = crate::api::brain_runs::catalog::capabilities(state)
                .await
                .map_err(internal)?;
            let all = super::catalog::descriptors(catalog);
            json!(all
                .iter()
                .filter(|cap| request.capability_ids.is_empty()
                    || request.capability_ids.contains(&cap.capability_id))
                .map(capability_metadata)
                .collect::<Vec<_>>())
        }
    };
    let events = reasons(state, id, snapshot.run.last_event_seq).await?;
    let mut grouped: BTreeMap<u32, Vec<Value>> = BTreeMap::new();
    for operation in &snapshot.operations {
        let child = state
            .fleet
            .assignment(&operation.execution_id)
            .await
            .map_err(internal)?;
        let mut value = json!(operation);
        value["execution_created"] = json!(child.is_some());
        if let Some(child) = child {
            let mut metadata = child.request.input["brain_scheduler"]["capability"].clone();
            if !metadata.is_object() {
                // Historical execution identity comes from its immutable assignment.
                metadata = json!({"capability_id":operation.capability_id,"kind":child.index.kind,"target":child.request.target});
            }
            value["capability"] = metadata;
        }
        grouped.entry(operation.round).or_default().push(value);
    }
    let rounds = grouped.into_iter().map(|(round, operations)| {
        let decisions: Vec<_> = events.iter().filter(|e| e.round == round).collect();
        json!({"run_id":id,"round":round,"status":round_status(&operations),"operations":operations,"decisions":decisions})
    }).collect::<Vec<_>>();
    Ok(
        json!({"schema_version":3,"objective":request.objective,"plan":assignment.request.input.get("plan"),
        "max_rounds":request.max_rounds,"input_names":request.inputs.keys().collect::<Vec<_>>(),
        "run":snapshot.run,"capability_ids":request.capability_ids,"capabilities":capabilities,"rounds":rounds}),
    )
}

async fn reasons(
    state: &Arc<AppState>,
    id: &str,
    watermark: u64,
) -> Result<Vec<BrainSchedulerEvent>, RpcReply> {
    let mut after = 0;
    let mut selected = Vec::new();
    while after < watermark {
        let reply = runs::call(state, id, "events", json!({"after":after,"limit":500})).await;
        if reply.status >= 300 {
            return Err(reply);
        }
        let events: Vec<BrainSchedulerEvent> =
            serde_json::from_value(reply.body["events"].clone()).map_err(internal)?;
        let next = events.last().map(|e| e.seq).unwrap_or(after);
        if next <= after {
            return Err(internal("scheduler event history is incomplete"));
        }
        selected.extend(
            events
                .into_iter()
                .filter(|e| e.seq <= watermark && e.reason_summary.is_some()),
        );
        after = next;
    }
    Ok(selected)
}

fn round_status(operations: &[Value]) -> &'static str {
    if operations.iter().any(|op| op["status"] == "error") {
        "failed"
    } else if operations.iter().any(|op| op["status"] == "cancelled") {
        "cancelled"
    } else if operations.iter().all(|op| op["status"] == "done") {
        "completed"
    } else {
        "running"
    }
}
