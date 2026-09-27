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
use std::{future::Future, pin::Pin};
use tokio_util::sync::CancellationToken;

type WorkloadFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(ExecutionStatus, Value)>> + Send + 'a>>;

pub(crate) async fn run(
    worker: &Worker,
    record: &Record,
    config: Config,
    cancel: CancellationToken,
    resume: bool,
) -> Result<(ExecutionStatus, Value)> {
    // Keep the large branch futures off Tokio's default worker stack.
    let execution: WorkloadFuture<'_> = match record.assignment.request.kind {
        ExecutionKind::Brain
            if record.assignment.request.input["schema_version"]
                == opencoder_core::brain::layered::LAYERED_SCHEMA_VERSION =>
        {
            Box::pin(crate::brain::v4::run(worker, record, config, cancel))
        }
        ExecutionKind::Brain => anyhow::bail!("unsupported brain schema; expected 7"),
        ExecutionKind::Agent | ExecutionKind::Maintenance | ExecutionKind::Operator => {
            Box::pin(agent::run(worker, record, config, cancel, resume))
        }
        ExecutionKind::Dag => Box::pin(dag::run(worker, record, config, cancel, resume)),
        ExecutionKind::Todos => Box::pin(todos::run(worker, record, config, cancel, resume)),
        ExecutionKind::Team => Box::pin(team::run(worker, record, config, cancel, resume)),
        ExecutionKind::System => anyhow::bail!("system team execution is retired"),
        ExecutionKind::Project => Box::pin(project::run(worker, record, cancel)),
    };
    let (status, result) = execution.await?;
    crate::brain::output::normalize(worker, record, status, result).await
}
