//! Model activation for one layer: strict JSON in, strict JSON out.
use anyhow::Result;
use opencoder_core::brain::layered::*;
use serde_json::json;

pub const PROMPT: &str = r#"You are an event-driven brain scheduler, schema_version 4.
The plan is a layered capability canvas: every node is already bound to exactly one capability.
Decide ONE layer, then stop. Return strict JSON only:
{"decision":"dispatch_layer","layer":<n>,"assignments":[{"node_id":"...","inputs":{"port":{"kind":"root","name":"..."}|{"kind":"execution","execution_id":"...","path":"/pointer"}|{"kind":"artifact","reference":"..."}},"reason":"..."}],"reason":"...","evidence_execution_ids":["..."]}
{"decision":"complete","reason":"...","evidence_execution_ids":["..."],"summary":"bounded final summary"}
{"decision":"fail","reason":"...","error_type":"..."}
Rules:
- "dispatch_layer" must set layer to the layer being asked about and cover every node of that layer exactly once.
- You may bind executions only from the direct upstream nodes listed for that node, and only successful ones.
- Do not invent nodes, capabilities, layers, inputs or evidence; every execution id you cite must appear in the context.
- Choose "complete" only after the final layer is done; "fail" only when the objective cannot be met."#;

/// Bounded instruction for one layer decision.
pub fn instruction(context: &LayeredContext) -> Result<String> {
    ensure_request(context)?;
    let nodes = context
        .nodes
        .iter()
        .map(|node| {
            json!({
                "node_id": node.node_id,
                "title": node.title,
                "instructions": node.instructions,
                "retry_max_attempts": node.retry_max_attempts,
                "capability": {
                    "capability_id": node.capability.capability_id,
                    "kind": node.capability.kind,
                    "target": node.capability.target,
                    "version": node.capability.version,
                    "input_desc": node.capability.input_desc,
                    "output_desc": node.capability.output_desc,
                    "required_inputs": node.capability.required_inputs,
                },
                "upstream": node.upstream,
                "downstream": node.downstream,
            })
        })
        .collect::<Vec<_>>();
    let instruction = json!({
        "schema_version": 4,
        "plan": {
            "title": context.request.plan.title,
            "objective": context.request.plan.objective,
            "total_layers": context.total_layers,
        },
        "layer": context.layer,
        "todo": context.todo,
        "root_inputs": context.request.inputs.keys().collect::<Vec<_>>(),
        "nodes": nodes,
    });
    Ok(serde_json::to_string_pretty(&instruction)?)
}

fn ensure_request(context: &LayeredContext) -> Result<()> {
    anyhow::ensure!(
        context.schema_version == LAYERED_SCHEMA_VERSION && !context.nodes.is_empty(),
        "layer context is not a v4 request"
    );
    Ok(())
}
