use crate::Worker;
use anyhow::Result;
use opencoder_core::fleet::*;

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
        if record.assignment.request.input["schema_version"] == 7 {
            super::v4::recover(worker, record).await?;
        }
    }
    Ok(())
}
