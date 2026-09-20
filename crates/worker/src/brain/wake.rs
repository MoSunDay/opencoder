use super::persistence;
use crate::Worker;
use anyhow::Result;
use opencoder_core::{brain::RunPhase, fleet::*};

/// This checks durable wake markers, not execution progress or the model.
/// It also recovers a root interrupted between committing an event and enqueue.
pub async fn recover_locked(worker: &Worker) -> Result<()> {
    let candidates: Vec<_> = worker
        .inner
        .journal
        .lock()
        .await
        .records
        .values()
        .filter(|r| {
            r.assignment.index.kind == ExecutionKind::Brain
                && matches!(
                    r.assignment.index.status,
                    ExecutionStatus::Idle
                        | ExecutionStatus::Interrupted
                        | ExecutionStatus::Cancelling
                )
        })
        .map(|record| record.assignment.index.id.clone())
        .collect();
    for id in candidates {
        let gate = worker.lifecycle_gate(&id).await;
        let _guard = gate.lock().await;
        if worker.inner.active.lock().await.contains_key(&id) {
            continue;
        }
        // Context delivery can enqueue the root while recovery waits for its
        // lifecycle gate. Re-read both status and annotations under the gate:
        // a stale Idle record must never overwrite the accepted Pending work.
        let record = worker.inner.journal.lock().await.records.get(&id).cloned();
        let Some(record) = record.filter(|record| {
            matches!(
                record.assignment.index.status,
                ExecutionStatus::Idle | ExecutionStatus::Interrupted | ExecutionStatus::Cancelling
            )
        }) else {
            continue;
        };
        if record.assignment.request.input["schema_version"] == 3 {
            super::v3::recover(worker, record).await?;
            continue;
        }
        let Some((_, run)) = persistence::load(worker, &id).await? else {
            continue;
        };
        if run.phase.terminal() {
            let status = match run.phase {
                RunPhase::Completed => ExecutionStatus::Done,
                RunPhase::Cancelled => ExecutionStatus::Cancelled,
                _ => ExecutionStatus::Error,
            };
            worker.inner.journal.lock().await.finalize(
                &id,
                status,
                serde_json::json!({"phase":run.phase,"deliverables":run.deliverables}),
                run.error,
            )?;
            continue;
        }
        if matches!(run.phase, RunPhase::Paused | RunPhase::Blocked) || run.candidate_plan.is_some()
        {
            continue;
        }
        if run.revision <= run.handled_revision
            && record.assignment.index.status != ExecutionStatus::Interrupted
        {
            continue;
        }
        let config = record
            .queue
            .as_ref()
            .map(|q| q.config.clone())
            .unwrap_or(worker.configuration()?);
        crate::operations::queue::enqueue(worker, record, config, true).await?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "wake/tests.rs"]
mod tests;
