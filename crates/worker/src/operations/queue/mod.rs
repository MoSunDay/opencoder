//! Node-owned durable pending work. Admission serializes enqueue and dispatch.
use crate::{journal::Record, Worker};
use anyhow::Result;
use opencoder_core::{fleet::*, Config};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct QueuedRun {
    pub sequence: u64,
    pub resume: bool,
    pub config: Config,
    #[serde(default)]
    pub command: Option<QueuedCommand>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct QueuedCommand {
    pub tail: String,
    pub body: serde_json::Value,
}

pub(crate) async fn enqueue(
    worker: &Worker,
    record: Record,
    config: Config,
    resume: bool,
) -> Result<Record> {
    enqueue_with_command(worker, record, config, resume, None).await
}

pub(crate) async fn enqueue_with_command(
    worker: &Worker,
    mut record: Record,
    config: Config,
    resume: bool,
    command: Option<QueuedCommand>,
) -> Result<Record> {
    let mut journal = worker.inner.journal.lock().await;
    if let Some(current) = journal.records.get(&record.assignment.index.id) {
        anyhow::ensure!(
            current.assignment.index.status == record.assignment.index.status
                && current.lifecycle.stop_intent == record.lifecycle.stop_intent,
            "execution changed before queue acceptance"
        );
    }
    let sequence = journal
        .records
        .values()
        .filter_map(|r| r.queue.as_ref().map(|q| q.sequence))
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("node queue sequence exhausted"))?;
    record.assignment.index.status = ExecutionStatus::Pending;
    record.lifecycle.stop_intent = None;
    record.queue = Some(Box::new(QueuedRun {
        sequence,
        resume,
        config,
        command,
    }));
    journal.save(record.clone())?;
    worker
        .inner
        .pending_runs
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    opencoder_session::loop_registry::notify_change();
    Ok(record)
}

/// Caller holds node admission; a slot is reserved before any workload starts.
pub(crate) async fn dispatch_locked(worker: &Worker) -> Result<()> {
    if worker.admission_error().is_some()
        || worker.inner.stopping.is_cancelled()
        || worker.inner.persistence_error.lock().unwrap().is_some()
        || worker
            .inner
            .state
            .project
            .require()?
            .persistence_error
            .lock()
            .unwrap()
            .is_some()
    {
        return Ok(());
    }
    let mut records: Vec<_> = worker
        .inner
        .journal
        .lock()
        .await
        .records
        .values()
        .filter(|r| r.assignment.index.status == ExecutionStatus::Pending && r.queue.is_some())
        .cloned()
        .collect();
    worker
        .inner
        .pending_runs
        .store(records.len() as u64, std::sync::atomic::Ordering::SeqCst);
    let order = worker.inner.scheduling.get().queue_order;
    records.sort_by(|a, b| {
        queue_cmp(
            order,
            a.queue.as_ref().unwrap().sequence,
            b.queue.as_ref().unwrap().sequence,
        )
    });
    for record in records {
        let Some(permit) = worker.try_slot() else {
            break;
        };
        let queued = record.queue.as_ref().unwrap().clone();
        let outcome = if let Some(command) = &queued.command {
            let id = record.assignment.index.id.clone();
            let gate = worker.lifecycle_gate(&id).await;
            let _guard = gate.lock().await;
            if worker.inner.journal.lock().await.records[&id]
                .assignment
                .index
                .status
                != ExecutionStatus::Pending
            {
                continue;
            }
            let reply = opencoder_core::harness::scope::with_settings(
                queued.config.agent.codex.clone(),
                opencoder_core::agent::scope::with_root(
                    queued.config.agent.agents_dir.clone(),
                    super::native(
                        worker,
                        "POST",
                        &format!("/api/sessions/{id}/{}", command.tail),
                        command.body.clone(),
                    ),
                ),
            )
            .await?;
            if reply.status >= 300 {
                worker.inner.journal.lock().await.finalize(
                    &id,
                    ExecutionStatus::Error,
                    record.result.clone(),
                    Some(format!("queued command rejected: {}", reply.body)),
                )?;
                worker
                    .inner
                    .pending_runs
                    .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                opencoder_session::loop_registry::notify_change();
                continue;
            }
            super::launch::launch_locked(
                worker.clone(),
                record,
                queued.config,
                permit,
                queued.resume,
            )
            .await?
        } else {
            super::launch::launch(worker.clone(), record, queued.config, permit, queued.resume)
                .await?
        };
        if matches!(outcome, super::launch::LaunchOutcome::Started) {
            worker
                .inner
                .pending_runs
                .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        } else if let super::launch::LaunchOutcome::NotRunnable(status) = outcome {
            tracing::debug!(?status, "pending execution changed before dispatch");
        }
    }
    Ok(())
}

pub(crate) fn start_scheduler(worker: &Worker) {
    let weak = std::sync::Arc::downgrade(&worker.inner);
    let stop = worker.inner.stopping.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
        loop {
            tokio::select! { _ = stop.cancelled() => break, _ = tick.tick() => {} }
            let Some(inner) = weak.upgrade() else {
                break;
            };
            let worker = Worker { inner };
            let _gate = worker.inner.admission.lock().await;
            if let Err(error) = dispatch_locked(&worker).await {
                tracing::error!(%error, "node pending dispatch failed");
                *worker.inner.persistence_error.lock().unwrap() =
                    Some(format!("pending dispatch: {error:#}"));
            }
        }
    });
}
