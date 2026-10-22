//! Content-addressed manifests retain earlier receipts after same-execution recovery.
use super::artifacts;
use crate::{journal::Record, Worker};
use anyhow::{ensure, Context, Result};
use opencoder_core::{
    brain::{ArtifactRef, BRAIN_EVIDENCE_MAX_BYTES},
    fleet::ExecutionRef,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(crate) fn publish(
    worker: &Worker,
    execution: &ExecutionRef,
    files: &[ArtifactRef],
) -> Result<ArtifactRef> {
    let value = json!({"schema_version":1,"execution":execution,"artifacts":files});
    let bytes = serde_json::to_vec_pretty(&value)?;
    let name = format!("artifacts-{:x}.json", Sha256::digest(&bytes));
    let dir = worker
        .inner
        .layout
        .execution_dir(execution.kind, &execution.id)?
        .join("brain-result");
    super::super::workspace::checked(&dir, &dir)?;
    std::fs::create_dir_all(&dir)?;
    // Freeze pretty JSON as well; the returned SHA covers the actual stored bytes.
    super::super::workspace::freeze(&dir.join(&name), &value)?;
    artifacts::reference(execution, &dir, "brain-result", None, &name)
}

pub(crate) fn bounded(
    worker: &Worker,
    record: &Record,
    output: &Value,
    files: Vec<ArtifactRef>,
) -> Result<Vec<ArtifactRef>> {
    let output_bytes = serde_json::to_vec(output)?.len();
    let index_bytes = if files.is_empty() {
        0
    } else {
        serde_json::to_vec(&files)?.len()
    };
    let files = if !files.is_empty() && output_bytes + index_bytes > BRAIN_EVIDENCE_MAX_BYTES {
        vec![publish(
            worker,
            &record.assignment.index.execution_ref(),
            &files,
        )?]
    } else {
        files
    };
    ensure!(output_bytes + if files.is_empty() { 0 } else { serde_json::to_vec(&files)?.len() } <= BRAIN_EVIDENCE_MAX_BYTES,
        "capability decision evidence exceeds {BRAIN_EVIDENCE_MAX_BYTES} bytes including artifact references");
    Ok(files)
}

pub(crate) async fn root(
    worker: &Worker,
    snapshot: &opencoder_core::brain::layered::LayeredSnapshot,
) -> Result<Vec<ArtifactRef>> {
    let journal = worker.inner.journal.lock().await;
    let record = journal
        .get(&snapshot.run.run_id)
        .await?
        .context("root execution missing")?;
    let mut files = Vec::new();
    for operation in opencoder_brain::layered::relevant_operations(snapshot) {
        if operation.status.successful() {
            let values = &record.annotations["layered_artifacts"][&operation.execution_id];
            if !values.is_null() {
                files.extend(serde_json::from_value::<Vec<ArtifactRef>>(values.clone())?);
            }
        }
    }
    let execution = record.assignment.index.execution_ref();
    drop(journal);
    if files.is_empty() {
        return Ok(files);
    }
    Ok(vec![publish(worker, &execution, &files)?])
}
