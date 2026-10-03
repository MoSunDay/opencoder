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
    let bound_inputs = match resolve_inputs(state, assignment, &request).await {
        Ok(inputs) => inputs,
        Err(reply) => return Ok(reply),
    };
    if let Err(error) =
        opencoder_brain::contracts::validate_inputs(&cap.required_inputs, &bound_inputs)
    {
        return Ok(RpcReply::error(422, error.to_string()));
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
            "id":op.execution_id, "schema_version":LAYERED_SCHEMA_VERSION,
            "node_id":root.index.node_id,
            "plan":{"id":cap.definition["plan_id"],"version":cap.definition["version"]},
            "inputs":child_inputs, "depth":run.depth + 1,
            "parent":{"run_id":run.run_id,"operation_id":op.operation_id,"node_id":op.node_id,"layer":op.layer}
        })).await);
    }
    let prompt = execution_prompt(cap, step, &bound_inputs)?;
    let mut input = json!({"schema_version":LAYERED_SCHEMA_VERSION,"brain_layered":{"run_id":run.run_id,"operation_id":op.operation_id,"layer":op.layer,"round":op.round,"activation":op.activation,"node_id":op.node_id,"attempt":op.attempt,"capability":super::view::capability_metadata(cap)},"bindings":bindings,"layered_inputs":bound_inputs,"prompt":prompt,"definition":cap.definition});
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

async fn resolve_inputs(
    state: &Arc<AppState>,
    assignment: &LayeredAssignment,
    request: &serde_json::Value,
) -> Result<serde_json::Map<String, serde_json::Value>, RpcReply> {
    let mut bound_inputs = serde_json::Map::new();
    for (name, binding) in &assignment.inputs {
        let value = match binding {
            BrainInputBinding::Value { value } => value.clone(),
            BrainInputBinding::Root { name: root_name } => request["inputs"]
                .get(root_name)
                .cloned()
                .ok_or_else(|| RpcReply::error(422, format!("unknown root input {root_name}")))?,
            BrainInputBinding::Artifact { reference } => request["artifacts"]
                .get(reference)
                .cloned()
                .ok_or_else(|| RpcReply::error(422, format!("unknown artifact {reference}")))?,
            BrainInputBinding::Execution { execution_id, path } => {
                let index = state
                    .fleet
                    .index(execution_id)
                    .await
                    .map_err(super::read::internal)?
                    .ok_or_else(|| {
                        RpcReply::error(
                            422,
                            format!(
                                "input {name}: referenced execution {execution_id} does not exist"
                            ),
                        )
                    })?;
                super::read::output(state, &index, path)
                    .await
                    .map_err(|reply| {
                        RpcReply::error(
                            reply.status,
                            format!(
                                "input {name}, execution {execution_id}, path {path:?}: {}",
                                reply.body
                            ),
                        )
                    })?
            }
        };
        bound_inputs.insert(name.clone(), value);
    }
    Ok(bound_inputs)
}

// Deliberately accepts no root plan or global reflection: the scheduler must
// translate rework into explicit inputs for this capability's bounded task.
fn execution_prompt(
    cap: &BrainCapabilityDescriptor,
    step: &LayeredNode,
    bound_inputs: &serde_json::Map<String, serde_json::Value>,
) -> Result<String> {
    Ok(format!(
        "You are executing one bounded capability task of one layer of the layered Brain canvas.\n\
         Capability: {} ({:?}), target: {}.\n\
         Capability description: {}\n\
         Input contract: {}\nOutput contract: {}\nRequired output fields: {}\n\
         Return concise evidence within {BRAIN_EVIDENCE_MAX_BYTES} JSON bytes, including every required field. \
         When required fields are configured, the final result must be a JSON object with those top-level fields. \
         Use artifact references for large files and reports. Other executions have separate workspaces; \
         deliver explicit code versions, patches or artifact references when another task needs your changes.\n\
         The Brain owns dispatching other nodes, layer barriers, collecting sibling execution IDs, \
         and deciding completion of the root objective. Those duties are not part of this node task. \
         Do not wait for sibling executions or repeat the root scheduling plan. \
         Use the bound inputs and the registered capability instructions to produce this capability's \
         result, then finish when that local result is verified. For a Team, completion and final_summary \
         describe only this Team's assigned result.\n\
         Step task:\n{}\nMilestone objective:\n{}\nSuccess criteria:\n{}\n\nLayered inputs:\n{}",
        cap.capability_id,
        cap.kind,
        cap.target,
        cap.summary,
        cap.input_desc,
        cap.output_desc,
        serde_json::to_string(&cap.required_outputs)?,
        step.title,
        step.objective,
        step.success_criteria,
        serde_json::to_string(bound_inputs)?
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_receives_local_criteria_and_explicit_remediation_inputs() {
        let cap: BrainCapabilityDescriptor = serde_json::from_value(json!({
            "capability_id":"team-review","kind":"team","target":"review",
            "summary":"Identify correctness problems in the assigned change",
            "input_desc":"Review the assigned patch","output_desc":"Review findings",
            "definition":{},"version":"1"
        }))
        .unwrap();
        let step: LayeredNode = serde_json::from_value(json!({
            "node_id":"review","title":"Review","layer":2,
            "objective":"Check the patch","success_criteria":"Report concrete findings",
            "capability_ids":["team-review"]
        }))
        .unwrap();
        let inputs = json!({"task":"Recheck the corrected null handling", "patch_id":"patch-2"});
        let prompt = execution_prompt(&cap, &step, inputs.as_object().unwrap()).unwrap();
        for expected in [
            "Identify correctness problems in the assigned change",
            "Check the patch",
            "Report concrete findings",
            "Recheck the corrected null handling",
            "patch-2",
        ] {
            assert!(prompt.contains(expected));
        }
        assert!(!prompt.contains("Root objective (context"));
        assert!(!prompt.contains("Reflection:"));
    }
}
