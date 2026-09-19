use super::state;
use crate::{journal::Record, Worker};
use anyhow::{Context, Result};
use opencoder_brain::scheduler;
use opencoder_core::{brain::*, fleet::*, message::now_ms, Config};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

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
        match worker.inner.state.store.brain_scheduler(id).await? {
            Some(s) => s,
            None => {
                worker
                    .inner
                    .state
                    .store
                    .commit_brain_scheduler(&scheduler::initialize(
                        id,
                        &state::request(record)?,
                        now_ms(),
                    )?)
                    .await?
            }
        }
    };
    if snapshot.run.phase != BrainSchedulerPhase::Deciding {
        return Ok(state::outcome(&snapshot));
    }
    // A root can race with the control wake immediately after admission;
    // without a durable context, leave it idle so that race cannot finalize
    // the root as an execution error. Node-local activation resumes only when
    // its context marker is present.
    if record.annotations["scheduler_context"].is_null() {
        return Ok(state::outcome(&snapshot));
    }
    let context: BrainSchedulerContext =
        serde_json::from_value(record.annotations["scheduler_context"].clone())?;
    anyhow::ensure!(
        context.generation == snapshot.run.generation,
        "stale scheduler context"
    );
    let stored = &record.annotations["scheduler_decision"];
    let decision = if stored["generation"] == context.generation {
        serde_json::from_value(stored["decision"].clone()).map_err(Into::into)
    } else {
        super::super::container::scheduler(worker, &config, &context, cancel.clone()).await
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
        let change = scheduler::decide(
            &current,
            &context.request,
            &context.capabilities,
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
                "scheduler_decision",
                json!({"generation":context.generation,"decision":decision}),
            )
            .await?;
            if let BrainSchedulerDecision::Dispatch { capabilities, .. } = decision {
                let intent = BrainDispatchIntent {
                    generation: change.run.generation,
                    operations: change
                        .operations
                        .iter()
                        .filter(|op| op.round == change.run.round)
                        .cloned()
                        .collect(),
                    items: capabilities,
                    capabilities: context.capabilities,
                };
                state::annotate(worker, id, "scheduler_intent", json!(intent)).await?;
            }
            change
        }
        Err(error) => {
            scheduler::block(&current, format!("scheduler decision: {error:#}"), now_ms())
        }
    };
    let next = worker
        .inner
        .state
        .store
        .commit_brain_scheduler(&change)
        .await?;
    state::annotate(worker, id, "scheduler_context", Value::Null).await?;
    Ok(state::outcome(&next))
}

/// Called under the node admission and root lifecycle gates. Only persisted
/// activation markers are recovered; running children are never inspected.
pub async fn recover(worker: &Worker, record: Record) -> Result<()> {
    let id = &record.assignment.index.id;
    let snapshot = worker.inner.state.store.brain_scheduler(id).await?;
    if let Some(snapshot) = &snapshot {
        state::settle(worker, snapshot).await?;
        if snapshot.run.phase != BrainSchedulerPhase::Deciding {
            return Ok(());
        }
        record
            .annotations
            .get("scheduler_context")
            .filter(|v| !v.is_null())
            .context("deciding run has no durable context")?;
    }
    let config = record
        .queue
        .as_ref()
        .map(|q| q.config.clone())
        .unwrap_or(worker.configuration()?);
    crate::operations::queue::enqueue(worker, record, config, true).await?;
    Ok(())
}
