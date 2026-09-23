//! Journal and projection access for the current layered root.
//!
//! The journal owns the frozen request and the finite activation markers; the
//! layered tables own the run, the operation indexes and the events only.
//! Layers are never stored and are always recomputed from the plan.
use crate::{journal::Record, Worker};
use anyhow::{Context, Result};
use opencoder_core::{
    brain::layered::{LayeredPhase, LayeredRequest, LayeredRunResult, LayeredSnapshot},
    fleet::ExecutionStatus,
};
use serde_json::{json, Value};

/// The durable projection of a root; a missing row is a programming error.
pub async fn load(worker: &Worker, id: &str) -> Result<LayeredSnapshot> {
    worker
        .inner
        .state
        .store
        .brain_layered(id)
        .await?
        .context("layered run missing")
}

/// Parse and validate a frozen layered request.
///
/// Control materializes `layered_request` at create time. The shorter `request`
/// object is accepted as the documented alias, and a bare v4 plan is the last
/// resort so a create endpoint that forwards the workbench body verbatim can
/// never silently fall back to v2 or v3.
pub fn parse_request(input: &Value) -> Result<LayeredRequest> {
    let value = ["layered_request", "request"]
        .iter()
        .filter_map(|key| input.get(*key))
        .find(|value| value["schema_version"] == 6)
        .cloned()
        .or_else(|| {
            let plan = input.get("plan")?;
            (plan["schema_version"] == 6).then(|| {
                json!({
                    "schema_version":6,
                    "plan":plan,
                    "inputs":input.get("inputs").cloned().unwrap_or_else(|| json!({})),
                })
            })
        })
        .context("layered request missing from the execution input")?;
    let request: LayeredRequest = serde_json::from_value(value)?;
    opencoder_brain::layered::validate_request(&request)?;
    Ok(request)
}

pub fn request(record: &Record) -> Result<LayeredRequest> {
    parse_request(&record.assignment.request.input)
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

/// Terminal payload written into the root journal. The completed layer, the
/// phase and the summary are the only things a parent plan binds to.
pub fn result(snapshot: &LayeredSnapshot) -> LayeredRunResult {
    LayeredRunResult {
        schema_version: 6,
        phase: snapshot.run.phase,
        layer: snapshot.run.layer,
        error: snapshot.run.error.clone(),
        scheduler_output: snapshot
            .run
            .summary
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
        scheduler_artifacts: json!([]),
    }
}

pub fn outcome(snapshot: &LayeredSnapshot) -> (ExecutionStatus, Value) {
    let status = match snapshot.run.phase {
        LayeredPhase::Completed => ExecutionStatus::Done,
        LayeredPhase::Failed => ExecutionStatus::Error,
        LayeredPhase::Cancelled => ExecutionStatus::Cancelled,
        _ => ExecutionStatus::Idle,
    };
    (status, json!(result(snapshot)))
}

/// Publish terminal root state into the journal, unless the root is running.
pub async fn settle(worker: &Worker, snapshot: &LayeredSnapshot) -> Result<()> {
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
