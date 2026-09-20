use crate::{
    api::brain_runs::catalog,
    api::{error_400, error_500, response},
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    response::Response,
    Json,
};
use opencoder_core::{brain::*, fleet::*};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
#[derive(Deserialize)]
pub struct Page {
    pub after: Option<u64>,
    pub limit: Option<u32>,
}
pub type Command = ExecutionCommand;

pub async fn create(State(state): State<Arc<AppState>>, Json(value): Json<Value>) -> Response {
    let mut request_value = value.clone();
    if let Some(object) = request_value.as_object_mut() {
        object.remove("id");
        object.remove("node_id");
    }
    let request: BrainSchedulerRequest = match serde_json::from_value(request_value) {
        Ok(v) => v,
        Err(e) => return error_400(e.to_string()),
    };
    if let Err(e) = opencoder_brain::scheduler::validate_request(&request) {
        return error_400(e.to_string());
    }
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("brain-{}", ulid::Ulid::new()));
    if !valid_id(&id) || !id.starts_with("brain-") {
        return error_400("invalid brain run id".into());
    }
    // Keep the request idempotent across retries and concurrent control-plane
    // callers. The fleet receipt owns the original intent; the scheduler
    // projection is only created by that owner.
    let _lock = match state.fleet.request_lock("brain-run", &id).await {
        Ok(lock) => lock,
        Err(error) => return error_500(error.to_string()),
    };
    let fingerprint = opencoder_core::token_hash(&value.to_string());
    match state
        .fleet
        .claim_request("brain-run", &id, &fingerprint)
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            let assignment = match state.fleet.assignment(&id).await {
                Ok(value) => value,
                Err(error) => return error_500(error.to_string()),
            };
            let Some(assignment) = assignment else {
                return response(RpcReply::error(409, "run id is still being prepared"));
            };
            if assignment.request.input["scheduler_intent"] == value {
                return response(RpcReply {
                    status: 202,
                    body: json!({
                        "schema_version": 3,
                        "run_id": id,
                        "execution": assignment.index,
                    }),
                });
            }
            return response(RpcReply::error(
                409,
                "run id was already accepted with a different intent",
            ));
        }
        Err(error) => return error_500(error.to_string()),
    }
    let input = json!({"schema_version":3,"scheduler_request":request,"scheduler_intent":value});
    let reply = crate::api::executions::submit(
        &state,
        CreateExecution {
            id: id.clone(),
            kind: ExecutionKind::Brain,
            target: None,
            input,
            node_id: value
                .get("node_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
    )
    .await;
    if reply.status >= 300 {
        return response(reply);
    }
    response(RpcReply {
        status: 202,
        body: json!({"schema_version":3,"run_id":id,"execution":reply.body}),
    })
}
pub async fn snapshot(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    response(super::super::runs::call(&state, &id, "snapshot", Value::Null).await)
}

/// Read-only presentation projection for the v3 scheduler workbench.
///
/// The projection combines the root request, scheduler indexes and current
/// capability metadata on demand. It deliberately omits definitions and all
/// child execution bodies; those remain available through their execution IDs.
pub async fn view(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let assignment = match state.fleet.assignment(&id).await {
        Ok(Some(assignment)) => assignment,
        Ok(None) => return response(RpcReply::error(404, "brain run not found")),
        Err(error) => return error_500(error.to_string()),
    };
    if assignment.request.input["schema_version"] != 3 {
        return response(RpcReply::error(409, SCHEDULER_MIGRATION));
    }
    let request: BrainSchedulerRequest =
        match serde_json::from_value(assignment.request.input["scheduler_request"].clone()) {
            Ok(request) => request,
            Err(error) => return error_500(format!("invalid scheduler request: {error}")),
        };
    let snapshot = super::super::runs::call(&state, &id, "snapshot", Value::Null).await;
    if snapshot.status >= 300 {
        return response(snapshot);
    }
    let snapshot: BrainSchedulerSnapshot = match serde_json::from_value(snapshot.body) {
        Ok(snapshot) => snapshot,
        Err(error) => return error_500(format!("invalid scheduler snapshot: {error}")),
    };
    let catalog = match catalog::capabilities(&state).await {
        Ok(catalog) => catalog,
        Err(error) => return error_500(error.to_string()),
    };
    let operation_ids: BTreeSet<_> = snapshot
        .operations
        .iter()
        .map(|operation| operation.capability_id.as_str())
        .collect();
    let capabilities: Vec<Value> = catalog
        .into_iter()
        .filter(|capability| {
            capability
                .get("id")
                .or_else(|| capability.get("capability_id"))
                .and_then(Value::as_str)
                .is_some_and(|id| operation_ids.contains(id))
        })
        .map(|capability| {
            json!({
                "capability_id": capability
                    .get("id")
                    .or_else(|| capability.get("capability_id"))
                    .cloned()
                    .unwrap_or(Value::Null),
                "kind": capability["kind"],
                "target": capability["target"],
                "version": capability["version"],
            })
        })
        .collect();
    let mut by_round: BTreeMap<u32, Vec<BrainOperation>> = BTreeMap::new();
    for operation in snapshot.operations.iter().cloned() {
        by_round.entry(operation.round).or_default().push(operation);
    }
    let rounds = by_round
        .into_iter()
        .map(|(round, operations)| {
            json!({
                "round": round,
                "status": round_status(&snapshot.run, round, &operations),
                "operations": operations,
            })
        })
        .collect::<Vec<_>>();
    response(RpcReply::ok(json!({
        "schema_version": 3,
        "objective": request.objective,
        "max_rounds": request.max_rounds,
        "input_names": request.inputs.keys().collect::<Vec<_>>(),
        "run": snapshot.run,
        "capabilities": capabilities,
        "rounds": rounds,
    })))
}

fn round_status(
    run: &BrainSchedulerRun,
    round: u32,
    operations: &[BrainOperation],
) -> &'static str {
    if operations
        .iter()
        .any(|operation| operation.status == BrainOperationStatus::Error)
    {
        return "failed";
    }
    if operations
        .iter()
        .any(|operation| operation.status == BrainOperationStatus::Cancelled)
    {
        return "cancelled";
    }
    if operations
        .iter()
        .all(|operation| operation.status == BrainOperationStatus::Done)
    {
        return "completed";
    }
    if round == run.round && run.phase == BrainSchedulerPhase::Ready {
        return "ready";
    }
    "running"
}
pub async fn events(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(page): Query<Page>,
) -> Response {
    response(
        super::super::runs::call(
            &state,
            &id,
            "events",
            json!({"after":page.after.unwrap_or(0),"limit":page.limit.unwrap_or(100).clamp(1,500)}),
        )
        .await,
    )
}
pub async fn round(
    State(state): State<Arc<AppState>>,
    Path((id, round)): Path<(String, u32)>,
) -> Response {
    response(super::super::runs::call(&state, &id, "round", json!({"round":round})).await)
}
pub async fn command(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<Command>,
) -> Response {
    if !matches!(body.action.as_str(), "pause" | "resume" | "cancel") {
        return error_400("supported commands: pause, resume, cancel".into());
    }
    let _lock = match state.fleet.request_lock("brain-control", &id).await {
        Ok(lock) => lock,
        Err(error) => return error_500(error.to_string()),
    };
    response(super::super::runs::call(&state, &id, &body.action, body.input).await)
}
