use crate::{journal::Record, Worker};
use anyhow::{Context, Result};
use opencoder_core::{brain::*, fleet::*};
use serde_json::{json, Value};

pub async fn load(worker: &Worker, id: &str) -> Result<BrainSchedulerSnapshot> {
    worker
        .inner
        .state
        .store
        .brain_scheduler(id)
        .await?
        .context("v3 scheduler missing")
}
pub fn request(record: &Record) -> Result<BrainSchedulerRequest> {
    Ok(serde_json::from_value(
        record.assignment.request.input["scheduler_request"].clone(),
    )?)
}
pub async fn annotate(worker: &Worker, id: &str, key: &str, value: Value) -> Result<()> {
    let mut journal = worker.inner.journal.lock().await;
    let mut record = journal
        .records
        .get(id)
        .context("root execution missing")?
        .clone();
    if !record.annotations.is_object() {
        record.annotations = json!({});
    }
    record.annotations[key] = value;
    journal.save(record)
}
pub fn outcome(snapshot: &BrainSchedulerSnapshot) -> (ExecutionStatus, Value) {
    let status = match snapshot.run.phase {
        BrainSchedulerPhase::Completed => ExecutionStatus::Done,
        BrainSchedulerPhase::Failed => ExecutionStatus::Error,
        BrainSchedulerPhase::Cancelled => ExecutionStatus::Cancelled,
        _ => ExecutionStatus::Idle,
    };
    (
        status,
        json!({"schema_version":3,"phase":snapshot.run.phase,"round":snapshot.run.round,"error":snapshot.run.error}),
    )
}
pub async fn settle(worker: &Worker, snapshot: &BrainSchedulerSnapshot) -> Result<()> {
    let id = &snapshot.run.run_id;
    if snapshot.run.phase.terminal() && !worker.inner.active.lock().await.contains_key(id) {
        let mut journal = worker.inner.journal.lock().await;
        let record = journal.records.get(id).context("root execution missing")?;
        if !record.assignment.index.status.terminal() {
            let (status, result) = outcome(snapshot);
            journal.finalize(id, status, result, snapshot.run.error.clone())?;
            opencoder_session::loop_registry::notify_change();
        }
    }
    Ok(())
}
