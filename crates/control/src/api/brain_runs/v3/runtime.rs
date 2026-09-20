//! The owning node is the only writer of scheduler state. Control resolves
//! catalog metadata and execution references for a finite node activation.
use crate::{api::brain_runs::runs, AppState};
use anyhow::{ensure, Context, Result};
use opencoder_core::{brain::*, fleet::*};
use serde_json::{json, Value};
use std::sync::Arc;

pub async fn wake(state: &Arc<AppState>, run_id: &str) -> Result<()> {
    let reply = runs::call(state, run_id, "snapshot", Value::Null).await;
    ensure!(reply.status < 300, "scheduler snapshot: {}", reply.body);
    let snapshot: BrainSchedulerSnapshot = serde_json::from_value(reply.body)?;
    if snapshot.run.phase != BrainSchedulerPhase::Ready {
        return Ok(());
    }
    let assignment = state
        .fleet
        .assignment(run_id)
        .await?
        .context("root assignment missing")?;
    let request: BrainSchedulerRequest =
        serde_json::from_value(assignment.request.input["scheduler_request"].clone())?;
    let capabilities = match super::catalog::available(state, &request).await {
        Ok(capabilities) => capabilities,
        Err(error) => {
            let reply = runs::call(
                state,
                run_id,
                "scheduler_block",
                json!({"generation": snapshot.run.generation, "error": error.to_string()}),
            )
            .await;
            ensure!(reply.status < 300, "scheduler block: {}", reply.body);
            return Ok(());
        }
    };
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
    Ok(())
}
