//! One catalog adapter shared by admission, saved plans and each activation.
use crate::{api::brain_runs::catalog, AppState};
use anyhow::{ensure, Result};
use opencoder_core::fleet::ExecutionKind;
use opencoder_core::{brain::layered::*, brain::*};
use serde_json::Value;
use std::sync::Arc;

pub fn descriptors(raw: Vec<Value>) -> Vec<BrainCapabilityDescriptor> {
    raw.into_iter()
        .filter_map(|value| {
            Some(BrainCapabilityDescriptor {
                capability_id: value
                    .get("id")
                    .or_else(|| value.get("capability_id"))?
                    .as_str()?
                    .into(),
                kind: serde_json::from_value(value.get("kind")?.clone()).ok()?,
                target: value["target"].as_str().unwrap_or("").into(),
                input_desc: value["input_desc"].as_str().unwrap_or("").into(),
                output_desc: value["output_desc"].as_str().unwrap_or("").into(),
                required_inputs: value["required_inputs"]
                    .as_array()
                    .map(|v| {
                        v.iter()
                            .filter_map(|x| x.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default(),
                definition: value.get("definition").cloned().unwrap_or(Value::Null),
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
    let all = descriptors(catalog::capabilities(state).await?);
    super::super::plan_capabilities::validate(&request.plan, request.depth, &all)?;
    let mut wanted: Vec<&str> = request
        .plan
        .nodes
        .iter()
        .map(|node| node.capability_id.as_str())
        .collect();
    wanted.sort_unstable();
    wanted.dedup();
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
