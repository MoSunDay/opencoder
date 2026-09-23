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
            BrainInputBinding::Value { value } => value.clone(),
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
    let pc_stage = opencoder_core::brain::pc_issue::stage(&cap.capability_id);
    if cap.kind == ExecutionKind::Brain {
        let mut child_inputs = cap.definition["plan"]["inputs"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        child_inputs.extend(bound_inputs);
        child_inputs.insert("brain_reflection".into(), json!(run.reflection));
        return Ok(super::api::submit(state.clone(), json!({
            "id":op.execution_id, "schema_version":5,
            "plan":{"id":cap.definition["plan_id"],"version":cap.definition["version"]},
            "inputs":child_inputs, "depth":run.depth + 1,
            "parent":{"run_id":run.run_id,"operation_id":op.operation_id,"node_id":op.node_id,"layer":op.layer}
        })).await);
    }
    let mut prompt = format!(
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
         Step task:\n{}\nMilestone objective:\n{}\nSuccess criteria:\n{}\nReflection:\n{}\n\nLayered inputs:\n{}",
        cap.capability_id,
        cap.kind,
        cap.target,
        cap.input_desc,
        cap.output_desc,
        request["plan"]["objective"].as_str().unwrap_or_default(),
        step.title,
        step.objective,
        step.success_criteria,
        run.reflection.as_deref().unwrap_or("Initial progression"),
        serde_json::to_string(&bound_inputs)?
    );
    if let Some(stage) = pc_stage {
        // Root problem/settings are authoritative, never model-rewritten bindings.
        bound_inputs.insert("problem".into(), request["inputs"]["problem"].clone());
        bound_inputs.insert("settings".into(), request["inputs"]["settings"].clone());
        let snapshot = super::read::snapshot(state, &run.run_id)
            .await
            .map_err(|reply| anyhow::anyhow!("PC issue history unavailable: {}", reply.body))?;
        let stage_index = opencoder_core::brain::pc_issue::STAGES
            .iter()
            .position(|s| *s == stage)
            .unwrap();
        let mut history = serde_json::Map::new();
        for prior in &opencoder_core::brain::pc_issue::STAGES[..stage_index] {
            let operation = snapshot
                .operations
                .iter()
                .filter(|item| item.node_id == *prior && item.status.successful())
                .max_by_key(|item| item.activation)
                .context("PC issue predecessor has no successful execution")?;
            let index = state
                .fleet
                .index(&operation.execution_id)
                .await?
                .context("PC issue predecessor index missing")?;
            let mut output = super::read::output(state, &index, "").await?;
            opencoder_core::brain::pc_issue::validate_output(prior, &output)?;
            if let Some(object) = output.as_object_mut() {
                object.remove("history");
            }
            history.insert(prior.to_string(), output);
        }
        if stage_index > 0 {
            bound_inputs.insert(
                "previous".into(),
                history[opencoder_core::brain::pc_issue::STAGES[stage_index - 1]].clone(),
            );
        }
        bound_inputs.insert("history".into(), json!(history));
        bound_inputs.insert("round".into(), json!(op.round));
        bound_inputs.insert("parent_execution_id".into(), json!(op.execution_id));
        let instructions = match stage {
            "impact" => include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../deploy/pc-issue/prompts/impact.md"
            )),
            "reproduce" => include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../deploy/pc-issue/prompts/reproduce.md"
            )),
            "repair" => include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../deploy/pc-issue/prompts/repair.md"
            )),
            "verify" => include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../deploy/pc-issue/prompts/verify.md"
            )),
            _ => include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../deploy/pc-issue/prompts/conclude.md"
            )),
        };
        prompt.push_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../deploy/pc-issue/prompts/common.md"
        )));
        prompt.push_str(instructions);
        prompt.push_str(&format!(
            "\nRoot run ID: {}\nAuthoritative stage inputs: {}",
            run.run_id,
            serde_json::to_string(&bound_inputs)?
        ));
    }
    let mut input = json!({"schema_version":5,"brain_layered":{"run_id":run.run_id,"operation_id":op.operation_id,"layer":op.layer,"round":op.round,"activation":op.activation,"node_id":op.node_id,"attempt":op.attempt,"capability":super::view::capability_metadata(cap)},"bindings":bindings,"layered_inputs":bound_inputs,"prompt":prompt,"definition":cap.definition});
    if let Some(stage) = pc_stage {
        input["pc_issue_stage"] = json!(stage);
        input["images"] =
            json!(super::super::attachments::images(state, &request["inputs"]["problem"]).await?);
    }
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
            node_id: pc_stage.map(|_| root.index.node_id.clone()),
        },
    )
    .await)
}
