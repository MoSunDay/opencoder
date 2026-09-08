mod admission;
mod command;
mod create;
mod dag_preflight;
mod launch;
mod maintenance;
mod project_admission;
mod query;
use crate::Worker;
use anyhow::Result;
pub(crate) use command::durable_stop;
pub(crate) use create::start;
use opencoder_core::fleet::*;
pub(crate) use query::native;

pub(crate) async fn handle(worker: &Worker, operation: NodeOperation) -> Result<RpcReply> {
    match operation {
        NodeOperation::Admission { command } => admission::update(worker, command).await,
        NodeOperation::Create { assignment } => create::create(worker, assignment).await,
        NodeOperation::Inspect { execution } => query::inspect(worker, &execution).await,
        NodeOperation::AcceptedRequest { execution } => {
            query::accepted_request(worker, &execution).await
        }
        NodeOperation::Events { execution, after } => {
            query::events(worker, &execution, after).await
        }
        NodeOperation::EventPayload { request } => query::event_payload(worker, request).await,
        NodeOperation::DetailField { request } => query::detail_field(worker, request).await,
        NodeOperation::Messages { execution, cursor } => {
            query::messages(worker, &execution, cursor).await
        }
        NodeOperation::TodoItems {
            execution,
            after_ordinal,
        } => query::todo_items(worker, &execution, after_ordinal).await,
        NodeOperation::ProjectRuns {
            execution,
            before_version,
        } => query::project_runs(worker, &execution, before_version).await,
        NodeOperation::TeamTurns {
            execution,
            after_turn,
        } => query::team_turns(worker, &execution, after_turn).await,
        NodeOperation::Artifact { request } => artifacts::read_request(worker, request).await,
        NodeOperation::Command { execution, command } => {
            command::command(worker, &execution, command).await
        }
        NodeOperation::Maintenance { command } => maintenance::run(worker, command).await,
    }
}

async fn validate_reference(worker: &Worker, execution: &ExecutionRef) -> Result<Option<RpcReply>> {
    if !valid_id(&execution.id) {
        return Ok(Some(RpcReply::error(400, "invalid execution id")));
    }
    if let Some(record) = worker
        .inner
        .journal
        .lock()
        .await
        .records
        .get(&execution.id)
        .cloned()
    {
        if record.assignment.index.kind != execution.kind
            || record.assignment.request.kind != execution.kind
        {
            return Ok(Some(RpcReply::error(409, "execution kind mismatch")));
        }
        return Ok(None);
    }
    if execution.kind == ExecutionKind::Project
        && worker
            .inner
            .state
            .project
            .require()?
            .projects
            .get_todo_run(&execution.id)
            .await?
            .is_some()
    {
        return Ok(None);
    }
    if execution.kind == ExecutionKind::Agent
        && worker
            .inner
            .state
            .store
            .get_session(&execution.id)
            .await?
            .is_some()
    {
        return Ok(None);
    }
    Ok(Some(RpcReply::error(404, "execution not found")))
}

mod fork;

mod artifacts;

#[cfg(test)]
mod admission_tests;
