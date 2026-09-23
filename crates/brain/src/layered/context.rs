use anyhow::{Context, Result};
use opencoder_core::brain::{layered::*, BrainCapabilityDescriptor};
use std::collections::BTreeMap;
pub fn layer_context(
    snapshot: &LayeredSnapshot,
    request: &LayeredRequest,
    descriptors: &[BrainCapabilityDescriptor],
    summaries: BTreeMap<String, String>,
    todo: Option<LayeredTodoSummary>,
) -> Result<LayeredContext> {
    let operations = relevant_operations(snapshot);
    let summaries = summaries
        .into_iter()
        .filter(|(id, _)| operations.iter().any(|op| &op.execution_id == id))
        .collect();
    let mut capabilities = BTreeMap::new();
    for id in request
        .plan
        .nodes
        .iter()
        .flat_map(|node| &node.capability_ids)
    {
        let cap = descriptors
            .iter()
            .find(|c| &c.capability_id == id)
            .with_context(|| format!("capability {id} unavailable"))?;
        capabilities.insert(id.clone(), cap.clone());
    }
    Ok(LayeredContext {
        run: Some(snapshot.run.clone()),
        schema_version: LAYERED_SCHEMA_VERSION,
        run_id: snapshot.run.run_id.clone(),
        generation: snapshot.run.generation,
        layer: snapshot.run.layer,
        total_layers: super::layers(&request.plan)?.len() as u32,
        request: request.clone(),
        capabilities: capabilities.into_values().collect(),
        todo,
        summaries,
        operations,
    })
}

/// Keep one visit per layer in the model context; all visits remain in the journal.
/// Invalidated suffix visits are diagnostic evidence, never automatically valid outputs.
pub fn relevant_operations(snapshot: &LayeredSnapshot) -> Vec<LayeredOperation> {
    let mut latest = BTreeMap::<u32, u64>::new();
    for op in &snapshot.operations {
        latest
            .entry(op.layer)
            .and_modify(|a| *a = (*a).max(op.activation))
            .or_insert(op.activation);
    }
    snapshot
        .operations
        .iter()
        .filter(|op| latest.get(&op.layer) == Some(&op.activation))
        .cloned()
        .collect()
}
