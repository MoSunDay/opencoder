//! Child outputs stay with the child execution.
//!
//! The parent root only ever receives a bounded summary or one JSON pointer
//! value while it assembles a layer context, so a node body can never grow the
//! root projection.
use super::parent;
use crate::{journal::Record, Worker};
use anyhow::{ensure, Context, Result};
use opencoder_core::fleet::*;
use serde_json::{json, Value};

/// Publish the managed child output for the parent's binding RPCs.
pub async fn normalize(
    worker: &Worker,
    record: &Record,
    status: ExecutionStatus,
    mut result: Value,
) -> Result<(ExecutionStatus, Value)> {
    if !matches!(status, ExecutionStatus::Done | ExecutionStatus::Idle) {
        return Ok((status, result));
    }
    let (mut output, artifacts) =
        super::super::output::native_output(worker, record, &result).await?;
    if let Some(text) = output.as_str() {
        if let Ok(structured) = serde_json::from_str(text) {
            output = structured;
        }
    }
    if !result.is_object() {
        result = json!({"native_result":result});
    }
    result["scheduler_output"] = output;
    result["scheduler_artifacts"] = json!(artifacts);
    Ok((ExecutionStatus::Done, result))
}

/// `layered_summary` returns the bounded text a parent prompt can carry;
/// `layered_output` resolves one JSON pointer into the child output.
pub async fn query(
    worker: &Worker,
    reference: &ExecutionRef,
    action: &str,
    input: Value,
) -> Result<RpcReply> {
    let journal = worker.inner.journal.lock().await;
    let record = journal
        .records
        .get(&reference.id)
        .context("execution missing")?;
    ensure!(
        record.assignment.index.kind == reference.kind,
        "execution kind mismatch"
    );
    let frozen = &record.assignment.request.input;
    let leaf = parent::leaf(frozen);
    ensure!(leaf || parent::root(frozen), "not a layered execution");
    ensure!(
        record.assignment.index.status == ExecutionStatus::Done,
        "execution has no successful terminal output"
    );
    // A leaf child binds to its own bounded output; a nested run binds to its
    // `LayeredRunResult` receipt, so a parent can read phase, layer or summary.
    let output = if leaf {
        record
            .result
            .get("scheduler_output")
            .or_else(|| record.result.get("layered_output"))
            .context("layered child output missing")?
    } else {
        &record.result
    };
    if action.ends_with("_summary") {
        let summary = match output.get("summary") {
            Some(value) => value,
            None if leaf => output,
            None => record
                .result
                .get("scheduler_output")
                .unwrap_or(&Value::Null),
        };
        let text = if let Some(text) = summary.as_str() {
            text.into()
        } else {
            serde_json::to_string(summary)?
        };
        let truncated = text.chars().count() > 4096;
        return Ok(RpcReply::ok(
            json!({"execution_id":reference.id,"summary":text.chars().take(4096).collect::<String>(),"truncated":truncated}),
        ));
    }
    let path = input["path"].as_str().context("output path required")?;
    let value = output.pointer(path).context("output path missing")?;
    ensure!(
        serde_json::to_vec(value)?.len() <= 256 * 1024,
        "bound output exceeds 256 KiB; bind an artifact reference"
    );
    Ok(RpcReply::ok(json!({"value":value})))
}
