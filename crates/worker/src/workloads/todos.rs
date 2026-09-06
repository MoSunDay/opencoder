use crate::{journal::Record, Worker};
use anyhow::Result;
use opencoder_core::{fleet::*, Config};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

pub(super) async fn run(
    worker: &Worker,
    record: &Record,
    config: Config,
    cancel: CancellationToken,
    resume: bool,
) -> Result<(ExecutionStatus, Value)> {
    let id = &record.assignment.index.id;
    let cancelled = cancel.clone();
    let runtime = opencoder_todos::Runtime {
        store: worker.inner.state.store.clone(),
        client: worker.client(&config)?,
        config,
        workdir: worker.inner.state.workdir.clone(),
        debug_root: None,
        cancel,
    };
    let workflow = worker.inner.state.store.get_todo_workflow(id).await?;
    let state = if resume && workflow.is_some() {
        worker
            .inner
            .journal
            .lock()
            .await
            .mark_todo_initialized(id)?;
        if workflow.is_some_and(|wf| wf.status == "running") {
            opencoder_todos::interrupt(
                &worker.inner.state.store,
                id,
                "explicit same-node recovery",
            )
            .await?;
        }
        runtime.resume(id).await?
    } else {
        let mut spec: opencoder_todos::WorkflowSpec =
            serde_json::from_value(record.assignment.definition.clone().unwrap())?;
        if let Some(prompt) = record.assignment.request.input["prompt"]
            .as_str()
            .filter(|p| !p.is_empty())
        {
            spec.objective = format!("{}\n执行要求：{prompt}", spec.objective);
        }
        let worker = worker.clone();
        let initialized_id = id.clone();
        runtime
            .run_new_with_id_observed(spec, id.clone(), move || async move {
                worker
                    .inner
                    .journal
                    .lock()
                    .await
                    .mark_todo_initialized(&initialized_id)
            })
            .await?
    };
    let status = match state.status.as_str() {
        "completed" => ExecutionStatus::Done,
        "failed" => ExecutionStatus::Error,
        _ => {
            if cancelled.is_cancelled() {
                ExecutionStatus::Interrupted
            } else {
                ExecutionStatus::Error
            }
        }
    };
    Ok((status, json!(state)))
}
