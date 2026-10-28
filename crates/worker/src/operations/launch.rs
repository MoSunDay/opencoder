use crate::{journal::Record, Worker};
use anyhow::Result;
use futures::FutureExt;
use opencoder_core::{fleet::ExecutionStatus, Config};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

pub(super) enum LaunchOutcome {
    Started,
    Running,
    NotRunnable(ExecutionStatus),
}

pub(super) async fn launch(
    worker: Worker,
    record: Record,
    config: Config,
    permit: tokio::sync::OwnedSemaphorePermit,
    resume: bool,
) -> Result<LaunchOutcome> {
    let id = record.assignment.index.id.clone();
    let gate = worker.lifecycle_gate(&id).await;
    let _guard = gate.lock().await;
    launch_locked(worker, record, config, permit, resume).await
}

pub(super) async fn launch_locked(
    worker: Worker,
    record: Record,
    config: Config,
    permit: tokio::sync::OwnedSemaphorePermit,
    resume: bool,
) -> Result<LaunchOutcome> {
    let id = record.assignment.index.id.clone();
    if worker.inner.active.lock().await.contains_key(&id) {
        return Ok(LaunchOutcome::Running);
    }
    let status = worker
        .inner
        .journal
        .lock()
        .await
        .summaries
        .get(&id)
        .ok_or_else(|| anyhow::anyhow!("execution not found"))?
        .index
        .status;
    if !(crate::lifecycle::can_launch(status, resume)
        || status == ExecutionStatus::Pending && record.queue.is_some())
    {
        return Ok(LaunchOutcome::NotRunnable(status));
    }
    let cancel = CancellationToken::new();
    worker
        .inner
        .journal
        .lock()
        .await
        .begin_run(&id, record.result.clone())
        .await?;
    worker
        .inner
        .active
        .lock()
        .await
        .insert(id.clone(), cancel.clone());
    opencoder_session::loop_registry::notify_change();
    let loop_guard = matches!(
        record.assignment.request.kind,
        opencoder_core::fleet::ExecutionKind::Agent
            | opencoder_core::fleet::ExecutionKind::Maintenance
    )
    .then(|| opencoder_session::loop_registry::LoopGuard::enter(&id));
    // Operator executions discover skills from their own frozen pool
    // (`<home>/.opencoder/skills`); every other kind keeps the node view.
    let skills_root = crate::brain::workdir::execution_skill_root(&worker, &record);
    let tasks = worker.inner.tasks.clone();
    tasks.spawn(async move {
        let ticket = record.queue.as_ref().and_then(|q| q.ticket.clone());
        let outcome = std::panic::AssertUnwindSafe(async {
            opencoder_core::harness::scope::with_execution(
                config.agent.codex.clone(),
                config.agent.runtime.clone(),
                opencoder_core::agent::scope::with_root(
                    config.agent.agents_dir.clone(),
                    Box::pin(opencoder_core::skill::with_execution(
                        skills_root.clone(),
                        crate::workloads::run(
                            &worker,
                            &record,
                            config.clone(),
                            cancel.clone(),
                            resume,
                        ),
                    )),
                ),
            )
            .await
        })
        .catch_unwind()
        .await
        .unwrap_or_else(|_| Err(anyhow::anyhow!("execution panicked")));
        let (status, result, error) = match outcome {
            Ok((status, result)) => {
                // DAG failures retain their result/artifact receipt as well as
                // the scheduler's terminal diagnostic in the execution index.
                let error = (status == ExecutionStatus::Error
                    && record.assignment.index.kind == opencoder_core::fleet::ExecutionKind::Dag)
                    .then(|| result["error"].as_str().map(str::to_owned))
                    .flatten();
                (status, result, error)
            }
            Err(error) => (
                if cancel.is_cancelled()
                    && record.assignment.index.kind == opencoder_core::fleet::ExecutionKind::Brain
                {
                    ExecutionStatus::Interrupted
                } else if cancel.is_cancelled() {
                    ExecutionStatus::Cancelled
                } else {
                    ExecutionStatus::Error
                },
                Value::Null,
                Some(format!("{error:#}")),
            ),
        };
        let gate = worker.lifecycle_gate(&id).await;
        let _guard = gate.lock().await;
        let persisted = worker
            .inner
            .journal
            .lock()
            .await
            .finalize(&id, status, result, error)
            .await;
        let terminal = persisted
            .as_ref()
            .is_ok_and(|status: &ExecutionStatus| status.terminal());
        if let Err(error) = persisted {
            tracing::error!(%id,%error,"could not persist terminal execution status");
            *worker.inner.persistence_error.lock().unwrap() =
                Some(format!("execution {id}: {error:#}"));
        } else if let Err(error) = worker.finish_slot(ticket.as_deref()).await {
            *worker.inner.persistence_error.lock().unwrap() =
                Some(format!("capacity completion {id}: {error:#}"));
        }
        // Terminal operator executions release their dedicated Linux
        // account (produced files handed back to the node user). Idempotent,
        // so a failed attempt can simply be retried later.
        #[cfg(target_os = "linux")]
        if record.assignment.index.kind == opencoder_core::fleet::ExecutionKind::Operator && terminal
        {
            if let Err(error) = crate::operations::operator_env::release_identity(
                &worker.inner.layout,
                record.assignment.index.kind,
                &record.assignment.index.id,
            ) {
                tracing::warn!(%id, %error, "operator identity release failed");
            }
        }
        worker.inner.active.lock().await.remove(&id);
        drop(loop_guard);
        drop(permit);
        // Release the task's own gate clone first so the entry below can be
        // collected; concurrent holders keep it alive via their clones.
        drop(_guard);
        drop(gate);
        crate::state::release_execution_state(&worker.inner, &id).await;
        opencoder_session::loop_registry::notify_change();
    });
    Ok(LaunchOutcome::Started)
}
