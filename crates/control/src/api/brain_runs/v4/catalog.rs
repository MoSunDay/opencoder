//! One catalog adapter shared by admission, saved plans and each activation.
use crate::{api::brain_runs::catalog, AppState};
use anyhow::{ensure, Result};
use opencoder_core::fleet::ExecutionKind;
use opencoder_core::{brain::layered::*, brain::*};
use serde_json::Value;
use std::sync::Arc;

pub fn descriptors(raw: &[Value]) -> Vec<BrainCapabilityDescriptor> {
    raw.iter()
        .filter_map(|value| {
            Some(BrainCapabilityDescriptor {
                capability_id: value
                    .get("id")
                    .or_else(|| value.get("capability_id"))?
                    .as_str()?
                    .into(),
                kind: serde_json::from_value(value.get("kind")?.clone()).ok()?,
                target: value["target"].as_str().unwrap_or("").into(),
                summary: value["summary"].as_str().unwrap_or("").into(),
                input_desc: value["input_desc"].as_str().unwrap_or("").into(),
                output_desc: value["output_desc"].as_str().unwrap_or("").into(),
                required_inputs: serde_json::from_value(
                    value
                        .get("required_inputs")
                        .cloned()
                        .unwrap_or(serde_json::json!([])),
                )
                .ok()?,
                definition: value.get("definition").cloned().unwrap_or(Value::Null),
                required_outputs: serde_json::from_value(
                    value
                        .get("required_outputs")
                        .cloned()
                        .unwrap_or(serde_json::json!([])),
                )
                .ok()?,
                version: value["version"].as_str().unwrap_or("").into(),
            })
        })
        .collect()
}

/// Node bindings are fixed at save time, so the catalog only has to resolve
/// every capability the plan references exactly once.
pub async fn available(
    state: &Arc<AppState>,
    request: &LayeredRequest,
) -> Result<Vec<BrainCapabilityDescriptor>> {
    let raw = catalog::capabilities(state).await?;
    let all = descriptors(&raw);
    let mut wanted: Vec<&str> = request
        .plan
        .nodes
        .iter()
        .flat_map(|node| node.capability_refs())
        .collect();
    wanted.sort_unstable();
    wanted.dedup();
    for id in &wanted {
        if let Some(reason) = raw
            .iter()
            .find(|c| c["id"] == *id && !c["unavailable_reason"].is_null())
        {
            anyhow::bail!(
                "node capability unavailable: {id}: {}",
                reason["unavailable_reason"]
            );
        }
    }
    super::super::plan_capabilities::validate(&request.plan, request.depth, &all)?;
    let capabilities: Vec<BrainCapabilityDescriptor> = all
        .into_iter()
        .filter(|c| {
            wanted.contains(&c.capability_id.as_str())
                && matches!(
                    c.kind,
                    ExecutionKind::Brain
                        | ExecutionKind::Agent
                        | ExecutionKind::Dag
                        | ExecutionKind::Team
                        | ExecutionKind::Todos
                        | ExecutionKind::Operator
                )
                && !c.target.trim().is_empty()
                && !c.input_desc.trim().is_empty()
                && !c.output_desc.trim().is_empty()
                && c.definition.is_object()
                && !c.version.trim().is_empty()
        })
        .collect();
    for id in &wanted {
        ensure!(
            capabilities
                .iter()
                .filter(|c| c.capability_id.as_str() == *id)
                .count()
                == 1,
            "node capability unavailable or ambiguous: {id}"
        );
    }
    ensure!(!capabilities.is_empty(), "no available capabilities");
    Ok(capabilities)
}
