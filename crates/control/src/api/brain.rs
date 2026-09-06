use super::{brain_dispatch as idem, error_400, error_404, error_500, response};
use crate::AppState;
use axum::{
    extract::{Path, State},
    response::Response,
    Json,
};
use opencoder_core::fleet::*;
use serde_json::{json, Value};
use std::sync::Arc;

pub async fn bind(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(target): Json<CapabilityTarget>,
) -> Response {
    if !matches!(
        target.kind,
        ExecutionKind::Agent | ExecutionKind::Team | ExecutionKind::Dag | ExecutionKind::Todos
    ) || target.target.trim().is_empty()
    {
        return error_400("capability target must name an agent, team or workflow".into());
    }
    match state.store.get_brain_capability(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return error_404("capability not found"),
        Err(error) => return error_500(error.to_string()),
    }
    match state
        .fleet
        .put_definition("capability_target", &id, &json!(target))
        .await
    {
        Ok(()) => response(RpcReply::ok(json!(target))),
        Err(error) => error_500(error.to_string()),
    }
}
pub async fn target(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.fleet.definition("capability_target", &id).await {
        Ok(value) => response(RpcReply::ok(json!({"target":value}))),
        Err(error) => error_500(error.to_string()),
    }
}
pub async fn dispatch(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> Response {
    let normalized = match idem::normalize(&body) {
        Ok(normalized) => normalized,
        Err(reply) => return response(reply),
    };
    let Some(request_id) = normalized.request_id.clone() else {
        return response(dispatch_unkeyed(&state, &normalized).await);
    };
    let mut gate = state.brain_gate.lock(&request_id).await;

    let mut existing = Vec::new();
    for (expected_kind, id) in idem::candidate_ids(&request_id) {
        match state.fleet.index(&id).await {
            Ok(Some(index)) if index.kind == expected_kind => existing.push(index),
            Ok(Some(_)) => return response(idem::conflict()),
            Ok(None) => {}
            Err(error) => return error_500(error.to_string()),
        }
    }
    if existing.len() > 1 {
        return response(RpcReply::error(
            409,
            "request_id has multiple execution candidates",
        ));
    }
    if let Some(index) = existing.first() {
        let accepted = state
            .hub
            .call(
                &index.node_id,
                NodeOperation::AcceptedRequest {
                    execution: index.execution_ref(),
                },
            )
            .await;
        if accepted.status == 404 {
            return response(RpcReply::error(
                503,
                "owning node has not confirmed this brain request",
            ));
        }
        if accepted.status != 200 {
            return response(accepted);
        }
        let receipt = match idem::receipt_from_accepted(
            &accepted.body,
            index,
            &request_id,
            &normalized.intent,
        ) {
            Ok(receipt) => receipt,
            Err(reply) => return response(reply),
        };
        gate.remove(&request_id);
        return response(idem::dispatch_reply(index, &receipt));
    }

    let planning_permit = {
        let _placement = state.placement.lock().await;
        match state.admission.enter().await {
            Ok(permit) => permit,
            Err(error) => return response(RpcReply::error(503, error)),
        }
    };

    let cached = match idem::claim(&mut gate, &request_id, &normalized.intent) {
        Ok(state) => state.prepared.clone(),
        Err(reply) => return response(reply),
    };
    let prepared = match cached {
        Some(prepared) => prepared,
        None => {
            let (result, target) = match plan(&state, &normalized).await {
                Ok(result) => result,
                Err(reply) => return response(reply),
            };
            let planner_model = normalized
                .intent
                .model
                .clone()
                .unwrap_or_else(|| state.brain.chat_model().to_string());
            let prepared = match idem::prepare(
                &normalized,
                &request_id,
                &result,
                target.kind,
                target.target,
                planner_model,
            ) {
                Ok(prepared) => prepared,
                Err(reply) => return response(reply),
            };
            idem::claim(&mut gate, &request_id, &normalized.intent)
                .expect("brain gate claim remains stable")
                .prepared = Some(prepared.clone());
            prepared
        }
    };
    drop(planning_permit);
    let execution = super::executions::submit(&state, prepared.request).await;
    if execution.status != 202 {
        return response(execution);
    }
    let index: ExecutionIndex = match serde_json::from_value(execution.body) {
        Ok(index) => index,
        Err(error) => return error_500(format!("invalid node acceptance: {error}")),
    };
    gate.remove(&request_id);
    response(idem::dispatch_reply(&index, &prepared.receipt))
}

pub async fn create_plan(
    State(state): State<Arc<AppState>>,
    Json(body): Json<crate::api_brain::PlanBody>,
) -> Response {
    let _permit = match planning_permit(&state).await {
        Ok(permit) => permit,
        Err(reply) => return response(reply),
    };
    crate::api_brain::create_plan(State(state), Json(body)).await
}

pub async fn preview(
    State(state): State<Arc<AppState>>,
    Json(body): Json<crate::api_brain::DispatchBody>,
) -> Response {
    let _permit = match planning_permit(&state).await {
        Ok(permit) => permit,
        Err(reply) => return response(reply),
    };
    crate::api_brain::dispatch(State(state), Json(body)).await
}

async fn planning_permit(state: &AppState) -> Result<crate::admission::AdmissionPermit, RpcReply> {
    let _placement = state.placement.lock().await;
    state
        .admission
        .enter()
        .await
        .map_err(|error| RpcReply::error(503, error))
}

async fn plan(
    state: &Arc<AppState>,
    normalized: &idem::NormalizedRequest,
) -> Result<(Value, CapabilityTarget), RpcReply> {
    let preview =
        crate::api_brain::dispatch(State(state.clone()), Json(normalized.preview())).await;
    let status = preview.status();
    let bytes = match axum::body::to_bytes(preview.into_body(), MAX_FRAME_BYTES).await {
        Ok(bytes) => bytes,
        Err(error) => return Err(RpcReply::error(500, error.to_string())),
    };
    let result: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(error) => return Err(RpcReply::error(500, error.to_string())),
    };
    if !status.is_success() {
        return Err(RpcReply {
            status: status.as_u16(),
            body: result,
        });
    }
    let Some(capability) = result["capability_id"]
        .as_str()
        .filter(|value| !value.is_empty())
    else {
        return Err(RpcReply::error(500, "brain result missing capability_id"));
    };
    let target: CapabilityTarget = match state
        .fleet
        .definition("capability_target", capability)
        .await
    {
        Ok(Some(value)) => match serde_json::from_value(value) {
            Ok(target) => target,
            Err(error) => return Err(RpcReply::error(500, error.to_string())),
        },
        Ok(None) => {
            return Err(RpcReply::error(
                400,
                format!("capability {capability} has no executable target"),
            ))
        }
        Err(error) => return Err(RpcReply::error(500, error.to_string())),
    };
    Ok((result, target))
}

async fn dispatch_unkeyed(state: &Arc<AppState>, normalized: &idem::NormalizedRequest) -> RpcReply {
    let planning_permit = match planning_permit(state).await {
        Ok(permit) => permit,
        Err(reply) => return reply,
    };
    let (mut result, target) = match plan(state, normalized).await {
        Ok(result) => result,
        Err(reply) => return reply,
    };
    let id = normalized
        .custom_id
        .clone()
        .unwrap_or_else(|| format!("{}-{}", target.kind.prefix(), ulid::Ulid::new()));
    let request = CreateExecution {
        id,
        kind: target.kind,
        target: Some(target.target),
        input: json!({
            "prompt": normalized.intent.situation,
            "capability_id": result["capability_id"],
        }),
        node_id: normalized.intent.node_id.clone(),
    };
    drop(planning_permit);
    let execution = super::executions::submit(state, request).await;
    if execution.status != 202 {
        return execution;
    }
    result["execution"] = execution.body;
    RpcReply {
        status: 202,
        body: result,
    }
}
