//! Detailed outputs remain with the child execution. Brain receives a summary
//! or a requested JSON pointer only when building a finite activation/input.
use crate::{journal::Record, Worker};
use anyhow::{ensure, Context, Result};
use opencoder_core::fleet::*;
use serde_json::{json, Value};

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
    ensure!(
        record
            .assignment
            .request
            .input
            .get("brain_scheduler")
            .is_some(),
        "not a scheduler child execution"
    );
    ensure!(
        record.assignment.index.status == ExecutionStatus::Done,
        "execution has no successful terminal output"
    );
    let output = record
        .result
        .get("scheduler_output")
        .context("execution output missing")?;
    if action == "scheduler_summary" {
        let summary = output.get("summary").unwrap_or(output);
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
