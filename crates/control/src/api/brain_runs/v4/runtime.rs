//! The owning node is the only writer of layered state. Control resolves
//! catalog metadata, bounded child summaries and the one-layer context for a
//! finite root activation.
use super::read;
use crate::{api::brain_runs::runs, AppState};
use anyhow::{ensure, Context, Result};
use opencoder_core::{brain::layered::*, brain::*};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};

pub async fn wake(state: &Arc<AppState>, run_id: &str) -> Result<Option<u64>> {
    let snapshot = read::snapshot(state, run_id)
        .await
        .map_err(|reply| anyhow::anyhow!("layered snapshot: {}", reply.body))?;
    if snapshot.run.phase != LayeredPhase::Ready {
        return Ok(None);
    }
    let assignment = state
        .fleet
        .assignment(run_id)
        .await?
        .context("root assignment missing")?;
    let request: LayeredRequest =
        serde_json::from_value(assignment.request.input["layered_request"].clone())?;
    let capabilities: Vec<BrainCapabilityDescriptor> =
        serde_json::from_value(assignment.request.input["frozen_capabilities"].clone())
            .context("frozen capability descriptors missing")?;
    let mut summaries = BTreeMap::new();
    let relevant = opencoder_brain::layered::relevant_operations(&snapshot);
    for operation in relevant.iter().filter(|op| op.status.terminal()) {
        let Some(index) = state.fleet.index(&operation.execution_id).await? else {
            ensure!(
                operation.status == LayeredOperationStatus::Error,
                "terminal execution index missing: {}",
                operation.execution_id
            );
            summaries.insert(
                operation.execution_id.clone(),
                "Execution admission failed before an execution index was created".into(),
            );
            continue;
        };
        let summary = read::summary(state, &index)
            .await
            .context("layered summary missing")?;
        summaries.insert(operation.execution_id.clone(), summary);
    }
    let context = context(state, &snapshot, &request, &capabilities, summaries).await?;
    let reply = runs::call(state, run_id, "layered_context", json!(context)).await;
    ensure!(reply.status < 300, "layered activation: {}", reply.body);
    if reply.body["stale"] == true {
        return Ok(None);
    }
    let admitted: LayeredSnapshot = serde_json::from_value(reply.body)?;
    Ok(Some(admitted.run.generation))
}

/// Dispatchable nodes for the next layer, or the empty context that only
/// permits the completion decision once every layer has been dispatched.
async fn context(
    state: &Arc<AppState>,
    snapshot: &LayeredSnapshot,
    request: &LayeredRequest,
    capabilities: &[BrainCapabilityDescriptor],
    summaries: BTreeMap<String, String>,
) -> Result<LayeredContext> {
    opencoder_brain::layered::layer_context(
        snapshot,
        request,
        capabilities,
        summaries,
        todo(state, request).await?,
    )
}

/// Bounded project-todo projection of the plan; a plan that names no todo, or
/// a todo that no longer exists, decides without it.
async fn todo(
    state: &Arc<AppState>,
    request: &LayeredRequest,
) -> Result<Option<LayeredTodoSummary>> {
    let Some(reference) = &request.plan.todo else {
        return Ok(None);
    };
    let Some(record) = state.projects.get_todo(&reference.id).await? else {
        return Ok(None);
    };
    Ok(Some(LayeredTodoSummary {
        id: record.id,
        title: record.title,
        status: record.status.as_str().into(),
        draft: record.draft.chars().take(4096).collect(),
    }))
}
