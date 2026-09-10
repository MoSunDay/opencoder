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
        .records
        .get(&id)
        .ok_or_else(|| anyhow::anyhow!("execution not found"))?
        .assignment
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
        .begin_run(&id, record.result.clone())?;
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
    let tasks = worker.inner.tasks.clone();
    tasks.spawn(async move {
        let outcome = std::panic::AssertUnwindSafe(async {
            opencoder_core::harness::scope::with_execution(
                config.agent.codex.clone(),
                config.agent.runtime.clone(),
                opencoder_core::agent::scope::with_root(
                    config.agent.agents_dir.clone(),
                    Box::pin(crate::workloads::run(
                        &worker,
                        &record,
                        config,
                        cancel.clone(),
                        resume,
                    )),
                ),
            )
            .await
        })
        .catch_unwind()
        .await
        .unwrap_or_else(|_| Err(anyhow::anyhow!("execution panicked")));
        let (status, result, error) = match outcome {
            Ok((status, result)) => (status, result, None),
            Err(error) => (
                if cancel.is_cancelled() {
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
            .finalize(&id, status, result, error);
        if let Err(error) = persisted {
            tracing::error!(%id,%error,"could not persist terminal execution status");
            *worker.inner.persistence_error.lock().unwrap() =
                Some(format!("execution {id}: {error:#}"));
        }
        worker.inner.active.lock().await.remove(&id);
        drop(loop_guard);
        drop(permit);
        opencoder_session::loop_registry::notify_change();
    });
    Ok(LaunchOutcome::Started)
}
