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
    // Targets are stored trimmed so bind and the agent grouping agree on one
    // agent key (a padded " act" would otherwise group apart from "act").
    let target = CapabilityTarget {
        target: target.target.trim().to_string(),
        ..target
    };
    match state.store.get_brain_capability(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return error_404("capability not found"),
        Err(error) => return error_500(error.to_string()),
    }
    // Lenient phantom gate: an agent binding naming an agent with no card
    // (custom or builtin) still succeeds — agents may be created after the
    // bind — but the mismatch is logged so a typo cannot silently group a
    // capability under a name nothing will ever resolve. Team/dag/todos
    // targets are free-form and stay unchecked.
    if target.kind == ExecutionKind::Agent
        && opencoder_core::agent::read_agent_meta(&target.target).is_none()
        && !opencoder_core::builtin_agents()
            .iter()
            .any(|a| a.name == target.target)
    {
        tracing::warn!(
            capability = %id,
            agent = %target.target,
            "capability bound to an unknown agent; keeping the bind (lenient gate)"
        );
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

/// Agent name → capability snapshots bound to that agent. Capabilities
/// without an agent binding (missing, unreadable, unparsable or non-agent
/// target) are skipped; BTreeMap keeps the agent order stable.
pub(crate) async fn agent_capability_groups(
    state: &AppState,
) -> anyhow::Result<Vec<(String, Vec<serde_json::Value>)>> {
    let mut groups: std::collections::BTreeMap<String, Vec<serde_json::Value>> =
        std::collections::BTreeMap::new();
    for capability in state.store.list_brain_capabilities().await? {
        let binding = match state
            .fleet
            .definition("capability_target", &capability.capability.id)
            .await
        {
            Ok(Some(value)) => value,
            Ok(None) => continue,
            Err(error) => {
                // Fail-visible: a store read error would otherwise freeze an
                // empty capability snapshot at team resolve with no trace.
                tracing::warn!(%error, capability = %capability.capability.id, "capability target read failed; treating capability as unbound");
                continue;
            }
        };
        let target: CapabilityTarget = match serde_json::from_value(binding) {
            Ok(target) => target,
            Err(_) => continue,
        };
        if target.kind != ExecutionKind::Agent || target.target.trim().is_empty() {
            continue;
        }
        groups
            .entry(target.target.trim().to_string())
            .or_default()
            .push(json!({
                "id": capability.capability.id,
                "summary": capability.capability.summary,
            }));
    }
    Ok(groups.into_iter().collect())
}

pub async fn agents(State(state): State<Arc<AppState>>) -> Response {
    match agent_capability_groups(&state).await {
        Ok(groups) => response(RpcReply::ok(json!({
            "agents": groups
                .into_iter()
                .map(|(agent, capabilities)| {
                    json!({"agent": agent, "capabilities": capabilities})
                })
                .collect::<Vec<_>>()
        }))),
        Err(error) => error_500(error.to_string()),
    }
}

/// GET /api/brain/playbooks — every persisted playbook, newest first.
/// Thin store passthrough (playbooks carry no embeddings), so the only
/// failure class is store I/O → 500.
pub async fn list_playbooks(State(state): State<Arc<AppState>>) -> Response {
    match state.store.list_brain_playbooks().await {
        Ok(playbooks) => response(RpcReply::ok(json!({ "playbooks": playbooks }))),
        Err(error) => error_500(error.to_string()),
    }
}

/// GET /api/brain/playbooks/:id — one playbook record. `spec_json` stays
/// opaque here (the brain crate owns decoding); 404 when absent.
pub async fn get_playbook(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.store.get_brain_playbook(&id).await {
        Ok(Some(record)) => response(RpcReply::ok(json!(record))),
        Ok(None) => error_404(&format!("brain playbook not found: {id}")),
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
    // An empty library is a normal first-use state: execute the request with
    // the default agent on the requested node. Store/planner failures remain
    // errors; an explicitly supplied plan is always resolved as requested.
    if normalized.intent.plan_id.is_none() {
        match state.store.list_brain_capabilities().await {
            Ok(capabilities) if capabilities.is_empty() => {
                return Ok((
                    json!({
                        "route": "default_agent", "plan_id": null, "capability_id": null,
                        "reason": "能力库为空，由默认 Agent 执行需求", "path": [],
                        "planned_fresh": false,
                    }),
                    default_target(),
                ));
            }
            Ok(_) => {}
            Err(error) => return Err(RpcReply::error(500, error.to_string())),
        }
    }
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
        Ok(None) => default_target(),
        Err(error) => return Err(RpcReply::error(500, error.to_string())),
    };
    Ok((result, target))
}

fn default_target() -> CapabilityTarget {
    CapabilityTarget {
        kind: ExecutionKind::Agent,
        target: "act".into(),
    }
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
