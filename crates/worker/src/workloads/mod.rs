pub(crate) mod agent;
mod agent_how;
pub(crate) mod agent_runc;
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
    let (status, result) = match record.assignment.request.kind {
        ExecutionKind::Brain if record.assignment.request.input["schema_version"] == 4 => {
            crate::brain::v4::run(worker, record, config, cancel).await
        }
        ExecutionKind::Brain if record.assignment.request.input["schema_version"] == 3 => {
            crate::brain::v3::run(worker, record, config, cancel).await
        }
        ExecutionKind::Brain => crate::brain::activate::run(worker, record, config, cancel).await,
        ExecutionKind::Agent | ExecutionKind::Maintenance | ExecutionKind::Operator => {
            agent::run(worker, record, config, cancel, resume).await
        }
        ExecutionKind::Dag => dag::run(worker, record, config, cancel, resume).await,
        ExecutionKind::Todos => todos::run(worker, record, config, cancel, resume).await,
        ExecutionKind::Team => team::run(worker, record, config, cancel, resume).await,
        ExecutionKind::System => anyhow::bail!("system team execution is retired"),
        ExecutionKind::Project => project::run(worker, record, cancel).await,
    }?;
    crate::brain::output::normalize(worker, record, status, result).await
}
