//! Node-local activation of a layered root: one layer decision per activation.
//!
//! The durable decision and its creation intent are journaled before the
//! operation indexes are published, so recovery replays the exact decision and
//! the exact attempt identities.
use super::state;
use crate::{journal::Record, Worker};
use anyhow::{ensure, Context, Result};
use opencoder_brain::layered;
use opencoder_core::{
    brain::{layered::*, BrainCapabilityDescriptor},
    fleet::*,
    message::now_ms,
    Config,
};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

/// The frozen descriptors of the layer being decided; the layer context carries
/// exactly the nodes a decision may dispatch.
fn catalog(context: &LayeredContext) -> Vec<BrainCapabilityDescriptor> {
    context.capabilities.clone()
}

pub async fn run(
    worker: &Worker,
    record: &Record,
    config: Config,
    cancel: CancellationToken,
) -> Result<(ExecutionStatus, Value)> {
    let id = &record.assignment.index.id;
    let gate = worker.lifecycle_gate(id).await;
    let snapshot = {
        let _guard = gate.lock().await;
        match worker.inner.state.store.brain_layered(id).await? {
            Some(snapshot) => snapshot,
            None => {
                worker
                    .inner
                    .state
                    .store
                    .commit_brain_layered(&layered::initialize(
                        id,
                        &state::request(record)?,
                        now_ms(),
                    )?)
                    .await?
            }
        }
    };
    if snapshot.run.phase != LayeredPhase::Deciding {
        return Ok(state::outcome(&snapshot));
    }
    // A root can race with the control wake immediately after admission;
    // without a durable context, leave it idle so that race cannot finalize the
    // root as an execution error. Node-local activation resumes only when its
    // context marker is present.
    if record.annotations["layered_context"].is_null() {
        return Ok(state::outcome(&snapshot));
    }
    let context: LayeredContext =
        serde_json::from_value(record.annotations["layered_context"].clone())?;
    ensure!(
        context.generation == snapshot.run.generation,
        "stale layered context"
    );
    let stored = &record.annotations["layered_decision"];
    let decision = if stored["generation"] == context.generation {
        serde_json::from_value(stored["decision"].clone()).map_err(Into::into)
    } else {
        super::super::container::layered(worker, &config, &context, cancel.clone()).await
    };
    if cancel.is_cancelled() {
        anyhow::bail!("brain activation interrupted");
    }
    let _guard = gate.lock().await;
    let current = state::load(worker, id).await?;
    if current.run.generation != context.generation {
        return Ok(state::outcome(&current));
    }
    let change = match decision.and_then(|decision| {
        let change = layered::decide(
            &current,
            &context.request,
            &catalog(&context),
            &decision,
            now_ms(),
        )?;
        Ok((decision, change))
    }) {
        Ok((decision, change)) => {
            // Persist the finite decision/creation intent before publishing its
            // operation indexes. Recovery replays this exact decision and IDs.
            state::annotate(
                worker,
                id,
                "layered_decision",
                json!({"generation":context.generation,"decision":decision}),
            )
            .await?;
            if let LayeredDecision::DispatchLayer { assignments, .. } = &decision {
                if change.run.phase == LayeredPhase::Waiting {
                    let intent = LayeredDispatchIntent {
                        generation: change.run.generation,
                        layer: change.run.layer,
                        operations: change
                            .operations
                            .iter()
                            .filter(|op| op.activation == change.run.activation)
                            .cloned()
                            .collect(),
                        assignments: assignments.clone(),
                        capabilities: catalog(&context),
                    };
                    state::annotate(worker, id, "layered_intent", json!(intent)).await?;
                }
            }
            change
        }
        Err(error) => layered::block(&current, format!("layered decision: {error:#}"), now_ms()),
    };
    let next = worker
        .inner
        .state
        .store
        .commit_brain_layered(&change)
        .await?;
    state::annotate(worker, id, "layered_context", Value::Null).await?;
    Ok(state::outcome(&next))
}

/// Called under the node admission and root lifecycle gates. Only persisted
/// activation markers are recovered; running children are never inspected.
pub async fn recover(worker: &Worker, record: Record) -> Result<()> {
    let id = &record.assignment.index.id;
    let snapshot = worker.inner.state.store.brain_layered(id).await?;
    if let Some(snapshot) = &snapshot {
        state::settle(worker, snapshot).await?;
        if snapshot.run.phase != LayeredPhase::Deciding {
            return Ok(());
        }
        record
            .annotations
            .get("layered_context")
            .filter(|value| !value.is_null())
            .context("deciding run has no durable context")?;
    }
    let config = record
        .queue
        .as_ref()
        .map(|queue| queue.config.clone())
        .unwrap_or(worker.configuration()?);
    crate::operations::queue::enqueue(worker, record, config, true).await?;
    Ok(())
}
