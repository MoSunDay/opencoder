//! Single adapter from a layer assignment to the existing execution API.
use crate::{api::executions, AppState};
use anyhow::{Context, Result};
use opencoder_core::{brain::layered::*, brain::*, fleet::*};
use serde_json::json;
use std::sync::Arc;

pub async fn dispatch(
    state: &Arc<AppState>,
    run: &LayeredRun,
    op: &LayeredOperation,
    assignment: &LayeredAssignment,
    cap: &BrainCapabilityDescriptor,
) -> Result<RpcReply> {
    let bindings = serde_json::to_value(&assignment.inputs)?;
    let root = state
        .fleet
        .assignment(&run.run_id)
        .await?
        .context("layered root assignment missing")?;
    let request = root.request.input["layered_request"].clone();
    let mut bound_inputs = serde_json::Map::new();
    for (name, binding) in &assignment.inputs {
        let value = match binding {
            BrainInputBinding::Root { name: root_name } => request["inputs"][root_name].clone(),
            BrainInputBinding::Artifact { reference } => request["artifacts"][reference].clone(),
            BrainInputBinding::Execution { execution_id, path } => {
                let index = state
                    .fleet
                    .index(execution_id)
                    .await?
                    .context("referenced execution index missing")?;
                super::read::output(state, &index, path).await?
            }
        };
        bound_inputs.insert(name.clone(), value);
    }
    let plan: LayeredPlan = serde_json::from_value(request["plan"].clone())?;
    let step = plan.node(&op.node_id).context("dispatch step missing")?;
    if cap.kind == ExecutionKind::Brain {
        let mut child_inputs = cap.definition["plan"]["inputs"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        child_inputs.extend(bound_inputs);
        return Ok(super::api::submit(state.clone(), json!({
            "id":op.execution_id, "schema_version":4,
            "plan":{"id":cap.definition["plan_id"],"version":cap.definition["version"]},
            "inputs":child_inputs, "depth":run.depth + 1,
            "parent":{"run_id":run.run_id,"operation_id":op.operation_id,"node_id":op.node_id,"layer":op.layer}
        })).await);
    }
    let prompt = format!(
        "You are executing one bounded capability task of one layer of the layered Brain canvas.\n\
         Capability: {} ({:?}), target: {}.\n\
         Input contract: {}\nOutput contract: {}\n\
         The Brain owns dispatching other nodes, layer barriers, collecting sibling execution IDs, \
         and deciding completion of the root objective. Those duties are not part of this node task. \
         Do not wait for sibling executions or repeat the root scheduling plan. \
         Use the bound inputs and the registered capability instructions to produce this capability's \
         result, then finish when that local result is verified. For a Team, completion and final_summary \
         describe only this Team's assigned result.\n\
         Root objective (context; apply only the portion assigned to this capability):\n{}\n\
         Step task:\n{}\n\nLayered inputs:\n{}",
        cap.capability_id,
        cap.kind,
        cap.target,
        cap.input_desc,
        cap.output_desc,
        request["plan"]["objective"].as_str().unwrap_or_default(),
        step.title,
        serde_json::to_string(&bound_inputs)?
    );
    let mut input = json!({"schema_version":4,"brain_layered":{"run_id":run.run_id,"operation_id":op.operation_id,"layer":op.layer,"node_id":op.node_id,"attempt":op.attempt,"capability":super::view::capability_metadata(cap)},"bindings":bindings,"layered_inputs":bound_inputs,"prompt":prompt,"definition":cap.definition});
    if op.execution_kind == ExecutionKind::Todos {
        input["spec"] = cap.definition.clone();
    }
    Ok(executions::submit(
        state,
        CreateExecution {
            id: op.execution_id.clone(),
            kind: op.execution_kind,
            target: Some(cap.target.clone()),
            input,
            node_id: None,
        },
    )
    .await)
}
