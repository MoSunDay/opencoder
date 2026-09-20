//! The owning node is the only writer of scheduler state. Control resolves
//! catalog metadata and execution references for a finite node activation.
use crate::{
    api::brain_runs::{catalog, runs},
    AppState,
};
use anyhow::{ensure, Context, Result};
use opencoder_core::{brain::*, fleet::*};
use serde_json::{json, Value};
use std::sync::Arc;

pub async fn wake(state: &Arc<AppState>, run_id: &str) -> Result<Option<u64>> {
    let reply = runs::call(state, run_id, "snapshot", Value::Null).await;
    ensure!(reply.status < 300, "scheduler snapshot: {}", reply.body);
    let snapshot: BrainSchedulerSnapshot = serde_json::from_value(reply.body)?;
    if snapshot.run.phase != BrainSchedulerPhase::Ready {
        return Ok(None);
    }
    let assignment = state
        .fleet
        .assignment(run_id)
        .await?
        .context("root assignment missing")?;
    let request: BrainSchedulerRequest =
        serde_json::from_value(assignment.request.input["scheduler_request"].clone())?;
    let raw = catalog::capabilities(state).await?;
    let mut all = Vec::new();
    for value in raw {
        let Some(id) = value
            .get("id")
            .or_else(|| value.get("capability_id"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let Ok(kind) = serde_json::from_value(value.get("kind").cloned().unwrap_or(Value::Null))
        else {
            continue;
        };
        all.push(BrainCapabilityDescriptor {
            capability_id: id.into(),
            kind,
            target: value
                .get("target")
                .and_then(Value::as_str)
                .unwrap_or("")
                .into(),
            input_desc: value
                .get("input_desc")
                .and_then(Value::as_str)
                .unwrap_or("")
                .into(),
            output_desc: value
                .get("output_desc")
                .and_then(Value::as_str)
                .unwrap_or("")
                .into(),
            required_inputs: value
                .get("required_inputs")
                .and_then(Value::as_array)
                .map(|v| {
                    v.iter()
                        .filter_map(|x| x.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            definition: value.get("definition").cloned().unwrap_or(Value::Null),
            version: value
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or("current")
                .into(),
        });
    }
    let capabilities = opencoder_brain::scheduler::prefilter(&all, &request);
    let mut summaries = std::collections::BTreeMap::new();
    for operation in snapshot.operations.iter().filter(|o| o.status.successful()) {
        let index = state
            .fleet
            .index(&operation.execution_id)
            .await?
            .context("child index missing")?;
        let reply = state
            .hub
            .call(
                &index.node_id,
                NodeOperation::Brain {
                    execution: index.execution_ref(),
                    action: "scheduler_summary".into(),
                    input: Value::Null,
                },
            )
            .await;
        ensure!(reply.status < 300, "scheduler summary: {}", reply.body);
        summaries.insert(
            operation.execution_id.clone(),
            reply.body["summary"]
                .as_str()
                .context("summary missing")?
                .into(),
        );
    }
    let context = BrainSchedulerContext {
        schema_version: 3,
        run_id: run_id.into(),
        generation: snapshot.run.generation,
        round: snapshot.run.round,
        request,
        capabilities,
        operations: snapshot.operations,
        summaries,
    };
    let reply = runs::call(state, run_id, "scheduler_context", json!(context)).await;
    ensure!(reply.status < 300, "scheduler activation: {}", reply.body);
    if reply.body["stale"] == true {
        return Ok(None);
    }
    let admitted: BrainSchedulerSnapshot = serde_json::from_value(reply.body)?;
    Ok(Some(admitted.run.generation))
}
