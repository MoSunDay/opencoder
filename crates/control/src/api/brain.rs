use super::{error_400, error_404, error_500, response};
use crate::AppState;
use axum::{
    extract::{Path, State},
    response::Response,
    Json,
};
use opencoder_core::fleet::*;
use serde_json::json;
use std::sync::Arc;

pub async fn bind(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(target): Json<CapabilityTarget>,
) -> Response {
    if let Err(error) = opencoder_brain::contracts::validate_fields(&target.required_inputs)
        .and_then(|_| opencoder_brain::contracts::validate_fields(&target.required_outputs))
    {
        return error_400(error.to_string());
    }
    if !matches!(
        target.kind,
        ExecutionKind::Agent
            | ExecutionKind::Team
            | ExecutionKind::Dag
            | ExecutionKind::Todos
            | ExecutionKind::Operator
    ) || target.target.trim().is_empty()
    {
        return error_400(
            "capability target must name an agent, team, workflow or operator".into(),
        );
    }
    // Targets are stored trimmed so bind and the agent grouping agree on one
    // agent key (a padded " act" would otherwise group apart from "act").
    let target = CapabilityTarget {
        target: target.target.trim().to_string(),
        ..target
    };
    let _lock = match state.fleet.request_lock("capability", &id).await {
        Ok(lock) => lock,
        Err(error) => return error_500(error.to_string()),
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

/// Fleet cleanup belongs to the Server; the standalone Web has no Fleet store.
pub async fn delete_capability(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let _catalog = match state.fleet.request_lock("brain-catalog", "registry").await {
        Ok(lock) => lock,
        Err(error) => return error_500(error.to_string()),
    };
    let _lock = match state.fleet.request_lock("capability", &id).await {
        Ok(lock) => lock,
        Err(error) => return error_500(error.to_string()),
    };
    let result = async {
        let references = super::brain_runs::maintenance::references(
            &state,
            super::brain_runs::maintenance::Removing::Capability(&id),
        )
        .await?;
        if !references.is_empty() {
            return Ok(response(RpcReply::error(
                409,
                format!("capability is referenced by: {}", references.join(", ")),
            )));
        }
        let exists = state.brain.get_capability(&id).await?.is_some()
            || state
                .fleet
                .definition("capability_target", &id)
                .await?
                .is_some()
            || state
                .fleet
                .definition("brain_capability_meta", &id)
                .await?
                .is_some();
        if !exists {
            return Ok(error_404(&format!("brain capability not found: {id}")));
        }
        // Retry also clears metadata left behind by a prior partial failure.
        state.brain.delete_capability(&id).await?;
        for kind in ["capability_target", "brain_capability_meta"] {
            state.fleet.delete_definition(kind, &id).await?;
        }
        Ok::<_, anyhow::Error>(response(RpcReply::ok(json!({"ok":true,"deleted":id}))))
    }
    .await;
    match result {
        Ok(reply) => reply,
        Err(error) => error_500(format!("delete brain capability: {error:#}")),
    }
}
