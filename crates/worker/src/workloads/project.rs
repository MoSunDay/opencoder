//! A project attempt is already durable when launch reaches this driver.
use crate::{journal::Record, Worker};
use anyhow::{Context, Result};
use opencoder_core::fleet::*;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

pub(super) async fn run(
    worker: &Worker,
    record: &Record,
    cancel: CancellationToken,
) -> Result<(ExecutionStatus, Value)> {
    let deps = worker.inner.state.project.require()?;
    let run_id = record.result["next_run_id"]
        .as_str()
        .context("accepted project run id missing")?;
    worker.inner.state.project.drive_reserved(run_id).await?;
    opencoder_session::loop_registry::notify_change();
    loop {
        if let Some(error) = deps.persistence_error.lock().unwrap().clone() {
            *worker.inner.persistence_error.lock().unwrap() = Some(error.clone());
            anyhow::bail!("{error}");
        }
        if cancel.is_cancelled() {
            worker.inner.state.project.cancel(run_id).await?;
        }
        let run = deps
            .projects
            .get_todo_run(run_id)
            .await?
            .context("project run disappeared")?;
        let status = match run.status {
            opencoder_store::ProjectTodoRunStatus::Running => {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            }
            opencoder_store::ProjectTodoRunStatus::Done => ExecutionStatus::Idle,
            opencoder_store::ProjectTodoRunStatus::Cancelled => ExecutionStatus::Cancelled,
            opencoder_store::ProjectTodoRunStatus::Failed => ExecutionStatus::Error,
        };
        // Keep the accepted snapshot for legacy resume; each run separately owns
        // its immutable input. No historical attempt lives solely in this result.
        let mut result = json!({"run_id":run_id,"active_run_id":run_id,"run":run});
        if let Some(snapshot) = record.result.get("next_snapshot") {
            result["next_snapshot"] = snapshot.clone();
        }
        return Ok((status, result));
    }
}
