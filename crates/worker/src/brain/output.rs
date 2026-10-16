use crate::{journal::Record, Worker};
use anyhow::{Context, Result};
use opencoder_core::{brain::*, fleet::*, Role};
use serde_json::Value;
pub(crate) mod artifacts;
#[cfg(not(windows))]
mod dag;
pub(crate) mod index;
mod legacy;

pub async fn normalize(
    worker: &Worker,
    record: &Record,
    status: ExecutionStatus,
    result: Value,
) -> Result<(ExecutionStatus, Value)> {
    if super::v4::parent::leaf(&record.assignment.request.input) {
        return super::v4::output::normalize(worker, record, status, result).await;
    }
    Ok((status, result))
}

pub(super) async fn native_output(
    worker: &Worker,
    record: &Record,
    result: &Value,
) -> Result<(Value, Vec<ArtifactRef>)> {
    if execution_io_version(&record.assignment.request.input)? == 0 {
        return legacy::native_output(worker, record, result).await;
    }
    let id = &record.assignment.index.id;
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
        #[cfg(not(windows))]
        ExecutionKind::Dag => return dag::output(worker, record),
        _ => anyhow::bail!("unsupported managed output kind"),
    };
    Ok((native, artifacts::workspace(worker, record)?))
}
