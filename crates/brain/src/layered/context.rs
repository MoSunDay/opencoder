//! One-layer context assembly: the model sees the current layer's nodes with
//! their frozen capability, their direct upstream executions and what the
//! downstream nodes expect.
use anyhow::{ensure, Result};
use opencoder_core::brain::layered::*;
use std::collections::BTreeMap;

pub fn layer_context(
    snapshot: &LayeredSnapshot,
    request: &LayeredRequest,
    descriptors: &[opencoder_core::brain::BrainCapabilityDescriptor],
    summaries: BTreeMap<String, String>,
    todo: Option<LayeredTodoSummary>,
) -> Result<LayeredContext> {
    let plan = &request.plan;
    let levels = super::layers(plan)?;
    let layer = snapshot.run.layer + 1;
    ensure!(
        layer as usize <= levels.len(),
        "every layer is already dispatched"
    );
    let level = &levels[layer as usize - 1];
    let mut nodes = vec![];
    for node_id in level {
        let node = plan.node(node_id).expect("layer node is a plan node");
        let capability = descriptors
            .iter()
            .find(|c| c.capability_id == node.capability_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("capability {} is unavailable", node.capability_id))?;
        let mut upstream = vec![];
        for edge in plan.edges.iter().filter(|e| &e.to == node_id) {
            let producer = plan.node(&edge.from).expect("edge node is a plan node");
            let op = super::terminal::latest_attempt(&snapshot.operations, &edge.from)
                .filter(|op| op.status.successful());
            upstream.push(LayeredUpstream {
                node_id: producer.node_id.clone(),
                title: producer.title.clone(),
                execution_id: op.map(|op| op.execution_id.clone()),
                summary: op.and_then(|op| summaries.get(&op.execution_id).cloned()),
            });
        }
        let mut downstream = vec![];
        for edge in plan.edges.iter().filter(|e| &e.from == node_id) {
            let consumer = plan.node(&edge.to).expect("edge node is a plan node");
            downstream.push(LayeredDownstream {
                node_id: consumer.node_id.clone(),
                title: consumer.title.clone(),
                needs: capability_inputs(descriptors, consumer),
            });
        }
        nodes.push(LayeredNodeContext {
            node_id: node.node_id.clone(),
            title: node.title.clone(),
            instructions: node.instructions.clone(),
            retry_max_attempts: node.retry.max_attempts,
            capability,
            upstream,
            downstream,
        });
    }
    Ok(LayeredContext {
        schema_version: LAYERED_SCHEMA_VERSION,
        run_id: snapshot.run.run_id.clone(),
        generation: snapshot.run.generation,
        layer,
        total_layers: levels.len() as u32,
        request: request.clone(),
        nodes,
        todo,
        summaries,
        operations: snapshot.operations.clone(),
    })
}

fn capability_inputs(
    descriptors: &[opencoder_core::brain::BrainCapabilityDescriptor],
    node: &LayeredNode,
) -> Vec<String> {
    descriptors
        .iter()
        .find(|c| c.capability_id == node.capability_id)
        .map(|c| c.required_inputs.clone())
        .unwrap_or_default()
}
