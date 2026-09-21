//! The owning node is the only writer of layered state. Control resolves
//! catalog metadata, bounded child summaries and the one-layer context for a
//! finite root activation.
use super::{catalog, read};
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
    let capabilities = match catalog::available(state, &request).await {
        Ok(capabilities) => capabilities,
        Err(error) => {
            let reply = runs::call(
                state,
                run_id,
                "layered_block",
                json!({"generation": snapshot.run.generation, "error": error.to_string()}),
            )
            .await;
            ensure!(reply.status < 300, "layered block: {}", reply.body);
            return Ok(None);
        }
    };
    let mut summaries = BTreeMap::new();
    for operation in snapshot
        .operations
        .iter()
        .filter(|op| op.status.successful())
    {
        let index = state
            .fleet
            .index(&operation.execution_id)
            .await?
            .context("child index missing")?;
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
    let total = opencoder_brain::layered::layers(&request.plan)?.len() as u32;
    let todo = todo(state, request).await?;
    if snapshot.run.layer < total {
        return opencoder_brain::layered::layer_context(
            snapshot,
            request,
            capabilities,
            summaries,
            todo,
        );
    }
    Ok(LayeredContext {
        schema_version: LAYERED_SCHEMA_VERSION,
        run_id: snapshot.run.run_id.clone(),
        generation: snapshot.run.generation,
        layer: snapshot.run.layer + 1,
        total_layers: total,
        request: request.clone(),
        nodes: vec![],
        todo,
        summaries,
        operations: snapshot.operations.clone(),
    })
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
