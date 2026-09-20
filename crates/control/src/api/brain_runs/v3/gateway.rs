use crate::{api::executions, AppState};
use anyhow::{Context, Result};
use opencoder_core::{brain::*, fleet::*};
use serde_json::json;
use std::sync::Arc;

/// Single adapter from a catalog operation to the existing execution API.
pub async fn dispatch(
    state: &Arc<AppState>,
    run: &BrainSchedulerRun,
    op: &BrainOperation,
    item: &BrainDispatchItem,
    cap: &BrainCapabilityDescriptor,
) -> Result<RpcReply> {
    let bindings = serde_json::to_value(&item.inputs)?;
    let root = state
        .fleet
        .assignment(&run.run_id)
        .await?
        .context("scheduler root assignment missing")?;
    let scheduler_request = root.request.input["scheduler_request"].clone();
    let mut bound_inputs = serde_json::Map::new();
    for (name, binding) in &item.inputs {
        let value = match binding {
            BrainInputBinding::Root { name: root_name } => {
                scheduler_request["inputs"][root_name].clone()
            }
            BrainInputBinding::Artifact { reference } => {
                scheduler_request["artifacts"][reference].clone()
            }
            BrainInputBinding::Execution { execution_id, path } => {
                let index = state
                    .fleet
                    .index(execution_id)
                    .await?
                    .context("referenced execution index missing")?;
                let reply = state
                    .hub
                    .call(
                        &index.node_id,
                        NodeOperation::Brain {
                            execution: index.execution_ref(),
                            action: "scheduler_output".into(),
                            input: json!({"path":path}),
                        },
                    )
                    .await;
                anyhow::ensure!(
                    reply.status < 300,
                    "referenced execution output unavailable: {}",
                    reply.body
                );
                reply.body["value"].clone()
            }
        };
        bound_inputs.insert(name.clone(), value);
    }
    let prompt = format!(
        "You are executing one bounded capability task for the Brain scheduler.\n\
         Capability: {} ({:?}), target: {}.\n\
         Input contract: {}\nOutput contract: {}\n\
         The Brain owns dispatching other capabilities, round barriers, collecting sibling execution IDs, \
         and deciding completion of the root objective. Those duties are not part of this child task. \
         Do not wait for sibling executions or repeat the root scheduling plan. \
         Use the bound inputs and the registered capability instructions to produce this capability's \
         result, then finish when that local result is verified. For a Team, completion and final_summary \
         describe only this Team's assigned result.\n\
         Root objective (context; apply only the portion assigned to this capability):\n{}\n\
         Scheduler inputs:\n{}",
        cap.capability_id,
        cap.kind,
        cap.target,
        cap.input_desc,
        cap.output_desc,
        scheduler_request["objective"].as_str().unwrap_or_default(),
        serde_json::to_string(&bound_inputs)?
    );
    let input = json!({"schema_version":3,"brain_scheduler":{"run_id":run.run_id,"operation_id":op.operation_id,"round":op.round},"bindings":bindings,"scheduler_inputs":bound_inputs,"prompt":prompt,"definition":cap.definition});
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
