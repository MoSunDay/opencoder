//! Child outputs stay with the child execution.
//!
//! The parent root only ever receives a bounded summary or one JSON pointer
//! value while it assembles a layer context, so a node body can never grow the
//! root projection.
use super::parent;
use crate::{journal::Record, Worker};
use anyhow::{ensure, Context, Result};
use opencoder_core::brain::BRAIN_EVIDENCE_MAX_BYTES;
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
    let required: Vec<String> = serde_json::from_value(
        record.assignment.request.input["brain_layered"]["capability"]
            .get("required_outputs")
            .cloned()
            .unwrap_or(json!([])),
    )?;
    if let Err(error) =
        opencoder_brain::contracts::validate_output(&required, &result["scheduler_output"])
    {
        result["scheduler_error"] = json!(error.to_string());
        return Ok((ExecutionStatus::Error, result));
    }
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
        record.assignment.index.status.terminal(),
        "execution has no terminal output"
    );
    // A leaf child binds to its own bounded output; a nested run binds to its
    // `LayeredRunResult` receipt, so a parent can read phase, layer or summary.
    let failure = json!({"status":record.assignment.index.status, "error":record.error, "result":record.result});
    let output = if record.assignment.index.status != ExecutionStatus::Done {
        &failure
    } else if leaf {
        record
            .result
            .get("scheduler_output")
            .or_else(|| record.result.get("layered_output"))
            .context("layered child output missing")?
    } else {
        &record.result
    };
    if action.ends_with("_summary") {
        let text = if record.assignment.index.status != ExecutionStatus::Done {
            failure_evidence(
                record.assignment.index.status,
                record.error.as_deref(),
                &record.result,
            )?
        } else if let Some(text) = output.as_str() {
            text.to_owned()
        } else {
            serde_json::to_string(output)?
        };
        // Never discard structured verdict fields in favor of a human summary.
        // Large leaf outputs are rejected during normalization, and a nested
        // plan's bounded receipt is returned intact as well.
        return Ok(RpcReply::ok(
            json!({"execution_id":reference.id,"summary":text,"truncated":false}),
        ));
    }
    let path = input["path"].as_str().context("output path required")?;
    let Some(value) = output.pointer(path) else {
        return Ok(RpcReply::error(
            422,
            format!("output path {path:?} missing"),
        ));
    };
    if serde_json::to_vec(value)?.len() > 256 * 1024 {
        return Ok(RpcReply::error(
            422,
            "bound output exceeds 256 KiB; bind an artifact reference",
        ));
    }
    Ok(RpcReply::ok(json!({"value":value})))
}

fn failure_evidence(
    status: ExecutionStatus,
    error: Option<&str>,
    result: &Value,
) -> Result<String> {
    let reason = result["scheduler_error"].as_str().or(error);
    let full = json!({"status":status,"error":reason,"result":result});
    let text = serde_json::to_string(&full)?;
    if text.len() <= BRAIN_EVIDENCE_MAX_BYTES {
        return Ok(text);
    }
    // A failed execution cannot satisfy a milestone. Preserve its diagnostic
    // and explicitly identify the omitted result, instead of hiding a verdict.
    Ok(serde_json::to_string(&json!({"status":status,
        "error":reason.map(|text| text.chars().take(1024).collect::<String>()),
        "result_truncated":true,"result_preview":result.to_string().chars().take(1024).collect::<String>()}))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_failure_keeps_the_contract_error_and_marks_the_omitted_body() {
        let result = json!({"scheduler_error":"required capability output field passed is missing", "log":"x".repeat(20000)});
        let evidence: Value =
            serde_json::from_str(&failure_evidence(ExecutionStatus::Error, None, &result).unwrap())
                .unwrap();
        assert_eq!(evidence["status"], "error");
        assert_eq!(evidence["error"], result["scheduler_error"]);
        assert_eq!(evidence["result_truncated"], true);
        assert!(evidence.to_string().len() < BRAIN_EVIDENCE_MAX_BYTES);
        let escaped = failure_evidence(
            ExecutionStatus::Error,
            Some(&"\0".repeat(3000)),
            &json!({"log":"🧪".repeat(6000)}),
        )
        .unwrap();
        assert!(escaped.len() <= BRAIN_EVIDENCE_MAX_BYTES);
    }
}
