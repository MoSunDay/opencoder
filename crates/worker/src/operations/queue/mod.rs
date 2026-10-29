//! Node-owned durable pending work. Admission serializes enqueue and dispatch.
use crate::{journal::Record, Worker};
use anyhow::Result;
use opencoder_core::{fleet::*, Config};
use serde::{Deserialize, Serialize};

/// Scheduler wakeup period after a cycle that started work.
const DISPATCH_INTERVAL_MS: u64 = 100;
/// Scheduler wakeup period while queued work stays unstartable (no free
/// slot or nothing runnable yet): 2 wakeups/sec.
const SATURATED_DISPATCH_INTERVAL_MS: u64 = 500;
/// Scheduler wakeup period while idle (2 wakeups/sec).
const IDLE_DISPATCH_INTERVAL_MS: u64 = 500;

/// Scheduler cadence for the cycle after `(started, pending)`:
/// - a cycle that started work keeps the fast period, because the slot
///   change may make further queued entries immediately runnable;
/// - a saturated cycle (pending work, nothing started) backs off: only a
///   slot release or a queue change can make progress, and both wake the
///   scheduler immediately through the loop registry;
/// - an idle cycle backs off the same way; a new enqueue wakes it up.
pub(crate) fn dispatch_delay(started: usize, pending: u64) -> std::time::Duration {
    if started > 0 {
        std::time::Duration::from_millis(DISPATCH_INTERVAL_MS)
    } else if pending > 0 {
        std::time::Duration::from_millis(SATURATED_DISPATCH_INTERVAL_MS)
    } else {
        std::time::Duration::from_millis(IDLE_DISPATCH_INTERVAL_MS)
    }
}

/// Wait one scheduler period or until the loop registry reports a change
/// (enqueue, slot release, admission or recovery transition). A notify that
/// lands mid-dispatch is observed by the watch receiver on the next call,
/// so backing off never delays newly runnable work. Returns false when the
/// worker is stopping.
async fn await_cycle(
    stop: &tokio_util::sync::CancellationToken,
    delay: std::time::Duration,
    changes: &mut tokio::sync::watch::Receiver<u64>,
) -> bool {
    tokio::select! {
        _ = stop.cancelled() => false,
        _ = tokio::time::sleep(delay) => true,
        _ = changes.changed() => true,
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct QueuedRun {
    #[serde(default)]
    pub ticket: Option<String>,
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
    let mut timing = super::admission::timing::Timing::new(&record.assignment.index.id, "enqueue");
    let mut journal = worker.inner.journal.lock().await;
    timing.mark("journal_lock");
    if let Some(current) = journal.summaries.get(&record.assignment.index.id) {
        anyhow::ensure!(
            current.index.status == record.assignment.index.status
                && current.stop_intent == record.lifecycle.stop_intent,
            "execution changed before queue acceptance"
        );
    }
    let sequence = journal
        .summaries
        .values()
        .filter_map(|r| r.queue.as_ref().map(|q| q.sequence))
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("node queue sequence exhausted"))?;
    record.assignment.index.status = ExecutionStatus::Pending;
    record.lifecycle.stop_intent = None;
    record.queue = Some(Box::new(QueuedRun {
        ticket: worker
            .inner
            .host_capacity
            .as_ref()
            .map(|_| ulid::Ulid::new().to_string()),
        sequence,
        resume,
        config,
        command,
    }));
    journal.save(record.clone())?;
    timing.mark("journal_save");
    if let Some(host) = &worker.inner.host_capacity {
        let ticket = record
            .queue
            .as_ref()
            .and_then(|q| q.ticket.as_deref())
            .ok_or_else(|| anyhow::anyhow!("hosted queue missing ticket"))?;
        host.store
            .enqueue_capacity(ticket, &record.assignment.index.id, &host.runtime_id)
            .await?;
    }
    worker
        .inner
        .pending_runs
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    opencoder_session::loop_registry::notify_change();
    Ok(record)
}

/// Transfer admission to a runtime-owned task until claim and launch finish.
/// Dropping an RPC response must not strand a claimed capacity ticket before
/// its execution is journaled as running. The returned guard preserves the
/// caller's serialization for any remaining acceptance/command work.
pub(crate) async fn dispatch_owned(
    worker: &Worker,
    gate: tokio::sync::OwnedMutexGuard<()>,
) -> Result<tokio::sync::OwnedMutexGuard<()>> {
    let (send, receive) = tokio::sync::oneshot::channel();
    let owned = worker.clone();
    worker.inner.tasks.spawn(async move {
        let result = dispatch_locked(&owned).await;
        if let Err(error) = &result {
            *owned.inner.persistence_error.lock().unwrap() =
                Some(format!("pending dispatch: {error:#}"));
        }
        // If the receiver disconnected, dropping the result releases admission
        // only after all claimed work was launched or explicitly failed.
        let _ = send.send(result.map(|_| gate));
    });
    receive.await?
}

/// Caller holds node admission; a slot is reserved before any workload
/// starts. Returns how many queued entries this cycle started, which drives
/// the scheduler cadence (see [`dispatch_delay`]).
pub(crate) async fn dispatch_locked(worker: &Worker) -> Result<usize> {
    let mut started = 0usize;
    let mut timing = super::admission::timing::Timing::new("", "dispatch");
    worker.reconcile_capacity().await?;
    timing.mark("capacity_reconciliation");
    crate::brain::wake::recover_locked(worker).await?;
    timing.mark("brain_recovery");
    super::todo::recover_locked(worker).await?;
    timing.mark("todo_recovery");
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
        return Ok(started);
    }
    let mut records: Vec<_> = worker
        .inner
        .journal
        .lock()
        .await
        .summaries
        .values()
        .filter(|r| r.index.status == ExecutionStatus::Pending && r.queue.is_some())
        .cloned()
        .collect();
    worker
        .inner
        .pending_runs
        .store(records.len() as u64, std::sync::atomic::Ordering::SeqCst);
    let order = if worker.inner.host_capacity.is_some() {
        QueueOrder::Fifo
    } else {
        worker.inner.scheduling.get().queue_order
    };
    records.sort_by(|a, b| {
        queue_cmp(
            order,
            a.queue.as_ref().unwrap().sequence,
            b.queue.as_ref().unwrap().sequence,
        )
    });
    timing.mark("queue_snapshot");
    for summary in records {
        let mut entry = super::admission::timing::Timing::new(&summary.index.id, "queue_entry");
        let Some(permit) = worker.try_slot() else {
            break;
        };
        let record = worker
            .inner
            .journal
            .lock()
            .await
            .get(&summary.index.id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("queued execution disappeared"))?;
        let queued = record.queue.as_ref().unwrap().clone();
        if let Some(host) = &worker.inner.host_capacity {
            let ticket = queued
                .ticket
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("hosted queue missing capacity ticket"))?;
            host.store
                .enqueue_capacity(ticket, &record.assignment.index.id, &host.runtime_id)
                .await?;
            if !host.store.claim_capacity(ticket, &host.runtime_id).await? {
                break;
            }
        }
        entry.mark("capacity_claim");
        let ticket = queued.ticket.clone();
        let outcome = if let Some(command) = &queued.command {
            let id = record.assignment.index.id.clone();
            let gate = worker.lifecycle_gate(&id).await;
            let _guard = gate.lock().await;
            if worker
                .inner
                .journal
                .lock()
                .await
                .get(&id)
                .await?
                .expect("execution exists")
                .assignment
                .index
                .status
                != ExecutionStatus::Pending
            {
                worker.finish_slot(ticket.as_deref()).await?;
                continue;
            }
            let reply = opencoder_core::harness::scope::with_execution(
                queued.config.agent.codex.clone(),
                queued.config.agent.runtime.clone(),
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
                worker
                    .inner
                    .journal
                    .lock()
                    .await
                    .finalize(
                        &id,
                        ExecutionStatus::Error,
                        record.result.clone(),
                        Some(format!("queued command rejected: {}", reply.body)),
                    )
                    .await?;
                worker
                    .inner
                    .pending_runs
                    .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                opencoder_session::loop_registry::notify_change();
                worker.finish_slot(ticket.as_deref()).await?;
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
            started += 1;
        } else if let super::launch::LaunchOutcome::NotRunnable(status) = outcome {
            worker.finish_slot(ticket.as_deref()).await?;
            tracing::debug!(?status, "pending execution changed before dispatch");
        } else {
            worker.finish_slot(ticket.as_deref()).await?;
        }
    }
    Ok(started)
}

pub(crate) fn start_scheduler(worker: &Worker) {
    let weak = std::sync::Arc::downgrade(&worker.inner);
    let stop = worker.inner.stopping.clone();
    // Track scheduler shutdown separately: an idle timer is not execution work
    // and must not prevent an empty Runtime from hibernating.
    worker.inner.background_tasks.spawn(async move {
        // Enqueue, slot release and recovery transitions notify the loop
        // registry, so new runnable work wakes the scheduler immediately;
        // the timer period is only the fallback between events. Cycles that
        // start nothing (saturated or idle) back off to 2 wakeups/sec.
        // dispatch_locked refreshes pending_runs each cycle. Start at the
        // active period so journal-recovered pending runs are picked up
        // promptly; a quiet runtime backs off after its first cycle.
        let mut changes = opencoder_session::loop_registry::subscribe();
        let mut delay = std::time::Duration::from_millis(DISPATCH_INTERVAL_MS);
        loop {
            if !await_cycle(&stop, delay, &mut changes).await {
                break;
            }
            let Some(inner) = weak.upgrade() else {
                break;
            };
            let worker = Worker { inner };
            // Shutdown holds admission while waiting for tracked cleanup.
            // A waiting scheduler must release its Worker capture on stop.
            let _gate = tokio::select! {
                biased;
                _ = stop.cancelled() => break,
                gate = worker.inner.admission.lock() => gate,
            };
            let started = match dispatch_locked(&worker).await {
                Ok(started) => started,
                Err(error) => {
                    tracing::error!(%error, "node pending dispatch failed");
                    *worker.inner.persistence_error.lock().unwrap() =
                        Some(format!("pending dispatch: {error:#}"));
                    0
                }
            };
            delay = dispatch_delay(
                started,
                worker.inner.pending_runs.load(std::sync::atomic::Ordering::SeqCst),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_delay_tiers_started_saturated_and_idle() {
        assert_eq!(
            dispatch_delay(2, 5),
            std::time::Duration::from_millis(DISPATCH_INTERVAL_MS),
            "a cycle that started work keeps the fast cadence"
        );
        assert_eq!(
            dispatch_delay(1, 0),
            std::time::Duration::from_millis(DISPATCH_INTERVAL_MS)
        );
        assert_eq!(
            dispatch_delay(0, 3),
            std::time::Duration::from_millis(SATURATED_DISPATCH_INTERVAL_MS),
            "pending work with no starts backs off"
        );
        assert_eq!(
            dispatch_delay(0, 0),
            std::time::Duration::from_millis(IDLE_DISPATCH_INTERVAL_MS),
            "an idle cycle backs off"
        );
    }

    #[tokio::test]
    async fn a_notify_landing_between_cycles_wakes_the_scheduler_immediately() {
        // A registry change that happens while the scheduler is dispatching
        // (or before its wait starts) must not wait out the backoff period.
        let mut changes = opencoder_session::loop_registry::subscribe();
        opencoder_session::loop_registry::notify_change();
        let stop = tokio_util::sync::CancellationToken::new();
        let woken = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            await_cycle(
                &stop,
                std::time::Duration::from_secs(60),
                &mut changes,
            ),
        )
        .await
        .expect("the queued notify must wake the scheduler at once");
        assert!(woken);
    }
}
