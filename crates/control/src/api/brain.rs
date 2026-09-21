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
