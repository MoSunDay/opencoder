use crate::{journal::Record, Worker};
use anyhow::{ensure, Context, Result};
use opencoder_core::{brain::*, fleet::*, Role};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;

pub async fn normalize(
    worker: &Worker,
    record: &Record,
    status: ExecutionStatus,
    mut result: Value,
) -> Result<(ExecutionStatus, Value)> {
    if record
        .assignment
        .request
        .input
        .get("brain_scheduler")
        .is_some()
    {
        return super::v3::output::normalize(worker, record, status, result).await;
    }
    if record.assignment.request.input.get("_brain").is_none()
        || !matches!(status, ExecutionStatus::Done | ExecutionStatus::Idle)
    {
        return Ok((status, result));
    }
    let action: ActionSpec =
        serde_json::from_value(record.assignment.request.input["_brain"]["action"].clone())?;
    let schema: DataSchema =
        serde_json::from_value(record.assignment.request.input["_brain"]["output_schema"].clone())?;
    let id = &record.assignment.index.id;
    let (native, mut artifacts) = native_output(worker, record, &result).await?;
    let value = project_output(native, &action)?;
    opencoder_brain::ontology::accepts(&schema, &value)?;
    ensure!(
        serde_json::to_vec(&value)?.len() <= 256 * 1024,
        "structured output exceeds 256 KiB; return artifact references instead"
    );
    let artifact_dir = worker
        .inner
        .layout
        .execution_dir(record.assignment.index.kind, id)?
        .join("brain-result");
    std::fs::create_dir_all(&artifact_dir)?;
    let artifact_path = artifact_dir.join("output.json");
    opencoder_core::atomic_write_json(&artifact_path, &value)?;
    artifacts.push(artifact(
        &record.assignment.index.execution_ref(),
        "brain-result",
        "output.json",
        &artifact_path,
    )?);
    let output = OutputEnvelope {
        value,
        artifacts,
        evidence: vec![format!("execution:{}", id)],
    };
    if !result.is_object() {
        result = json!({"native_result":result});
    }
    result["brain_output"] = json!(output);
    Ok((ExecutionStatus::Done, result))
}

pub(super) async fn native_output(
    worker: &Worker,
    record: &Record,
    result: &Value,
) -> Result<(Value, Vec<ArtifactRef>)> {
    let id = &record.assignment.index.id;
    let mut artifacts = vec![];
    let native = match record.assignment.index.kind {
        ExecutionKind::Agent | ExecutionKind::Operator => {
            let messages = worker.inner.state.store.load_messages(id).await?;
            let final_message = messages
                .iter()
                .rev()
                .find(|m| m.role == Role::Assistant && !m.text().trim().is_empty())
                .context("agent completed without a final output")?;
            Value::String(final_message.text())
        }
        ExecutionKind::Team => result
            .get("final_summary")
            .filter(|v| !v.is_null())
            .cloned()
            .context("team completed without final_summary")?,
        ExecutionKind::Todos => {
            let mut outputs = serde_json::Map::new();
            for (name, todo) in result["todos"]
                .as_object()
                .context("TODO result missing items")?
            {
                let candidate = &todo["candidate"];
                let value = candidate["result"]
                    .as_str()
                    .or(candidate["summary"].as_str())
                    .context("TODO completed without accepted output")?;
                outputs.insert(name.clone(), Value::String(value.into()));
            }
            Value::Object(outputs)
        }
        ExecutionKind::Dag => {
            let root = Path::new(
                result["artifact_root"]
                    .as_str()
                    .context("DAG artifact root missing")?,
            );
            let mut outputs = serde_json::Map::new();
            let definition = record
                .assignment
                .definition
                .as_ref()
                .context("DAG definition missing")?;
            for step in definition.get("spec").unwrap_or(definition)["steps"]
                .as_array()
                .context("DAG steps missing")?
            {
                let name = step["name"]
                    .as_str()
                    .or(step["id"].as_str())
                    .context("DAG step id missing")?;
                let dir = root.join(name);
                if !dir.join("meta.json").exists() {
                    continue;
                }
                let structured: Value =
                    serde_json::from_slice(&std::fs::read(dir.join("output.json"))?)?;
                let value = if structured.is_null() {
                    Value::String(std::fs::read_to_string(dir.join("output.txt"))?)
                } else {
                    structured
                };
                outputs.insert(name.into(), value);
                for file in ["output.json", "output.txt", "meta.json"] {
                    artifacts.push(artifact(
                        &record.assignment.index.execution_ref(),
                        name,
                        file,
                        &dir.join(file),
                    )?);
                }
            }
            Value::Object(outputs)
        }
        _ => anyhow::bail!("unsupported managed output kind"),
    };
    Ok((native, artifacts))
}

pub fn project_output(mut value: Value, action: &ActionSpec) -> Result<Value> {
    if action.output_mode == OutputMode::Json {
        if let Some(text) = value.as_str() {
            value = serde_json::from_str(text).context("execution did not return valid JSON")?;
        }
    }
    value = value
        .pointer(&action.output_pointer)
        .cloned()
        .context("declared output pointer is absent")?;
    match action.output_mode {
        OutputMode::Json => {
            if let Some(text) = value.as_str() {
                serde_json::from_str(text)
                    .context("declared JSON output contains invalid JSON text")
            } else {
                Ok(value)
            }
        }
        OutputMode::Text => Ok(match value {
            Value::String(_) => value,
            _ => Value::String(serde_json::to_string(&value)?),
        }),
    }
}
fn artifact(execution: &ExecutionRef, step: &str, file: &str, path: &Path) -> Result<ArtifactRef> {
    let bytes = std::fs::read(path)?;
    Ok(ArtifactRef {
        execution: execution.clone(),
        step: step.into(),
        file: file.into(),
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        bytes: bytes.len() as u64,
    })
}
