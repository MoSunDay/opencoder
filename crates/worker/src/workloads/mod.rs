mod agent;
mod dag;
mod project;
mod team;
mod todos;
use crate::{journal::Record, Worker};
use anyhow::Result;
use opencoder_core::{fleet::*, Config};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

pub(crate) async fn run(
    worker: &Worker,
    record: &Record,
    config: Config,
    cancel: CancellationToken,
    resume: bool,
) -> Result<(ExecutionStatus, Value)> {
    match record.assignment.request.kind {
        ExecutionKind::Agent | ExecutionKind::Maintenance => {
            agent::run(worker, record, config, cancel, resume).await
        }
        ExecutionKind::Dag => dag::run(worker, record, config, cancel, resume).await,
        ExecutionKind::Todos => todos::run(worker, record, config, cancel, resume).await,
        ExecutionKind::Team => team::run(worker, record, config, cancel, resume).await,
        ExecutionKind::System => anyhow::bail!("system team execution is retired"),
        ExecutionKind::Project => project::run(worker, record, cancel).await,
    }
}
