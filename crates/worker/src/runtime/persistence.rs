//! Self-healing persistence latch. A store/NFS failure used to latch node
//! dispatch forever (the latch was write-only); failures now schedule
//! bounded exponential backoff probes and a fully successful dispatch cycle
//! proves persistence works again and clears both latches.
use crate::Worker;
use std::time::{Duration, Instant};

/// Delay before the first persistence probe after a failure.
pub(crate) const RETRY_INITIAL: Duration = Duration::from_secs(1);
/// Probe delay ceiling: a permanently broken store must never spin the loop.
pub(crate) const RETRY_MAX: Duration = Duration::from_secs(60);

/// Backoff schedule state for the persistence latch. `next_probe_at` is
/// `None` while no worker-side failure is waiting out its retry window.
#[derive(Default)]
pub(crate) struct PersistenceRetry {
    pub attempts: u32,
    pub next_probe_at: Option<Instant>,
}

/// Pure backoff schedule: 1s, 2s, 4s, ... capped at [`RETRY_MAX`]. The
/// shift exponent is capped at 6, so extreme attempt counts cannot overflow.
pub(crate) fn backoff(attempts: u32) -> Duration {
    let exponent = attempts.saturating_sub(1).min(6);
    RETRY_MAX.min(RETRY_INITIAL * (1u32 << exponent))
}

/// Latch a persistence failure: remember the reason (node snapshot, Create
/// rejection) and schedule the next dispatch probe with bounded backoff.
pub(crate) fn record(worker: &Worker, message: String) {
    let delay = {
        let mut retry = worker.inner.persistence_retry.lock().unwrap();
        retry.attempts = retry.attempts.saturating_add(1);
        let delay = backoff(retry.attempts);
        retry.next_probe_at = Some(Instant::now() + delay);
        delay
    };
    *worker.inner.persistence_error.lock().unwrap() = Some(message.clone());
    tracing::error!(error = %message, backoff = ?delay, "node persistence latched; dispatch probes back off");
}

/// True while the project-side latch is set. An uninitialized project
/// service holds no latch (see [`active`] for the conservative variant).
fn deps_latched(worker: &Worker) -> bool {
    worker
        .inner
        .state
        .project
        .require()
        .map(|deps| deps.persistence_error.lock().unwrap().is_some())
        .unwrap_or(false)
}

/// True while a latched failure is still inside its backoff window, so the
/// dispatch loop must not touch the store yet. A deps-only latch without a
/// scheduled probe (never recorded through [`record`]) is immediately
/// probeable: one dispatch cycle either clears it or re-latches it.
pub(crate) fn blocked(worker: &Worker) -> bool {
    let own = worker.inner.persistence_error.lock().unwrap().is_some();
    if !own && !deps_latched(worker) {
        return false;
    }
    match worker.inner.persistence_retry.lock().unwrap().next_probe_at {
        // No probe scheduled: only a deps-only latch may sit here, and it is
        // probeable now; a worker-side latch always carries a deadline.
        None => own,
        Some(deadline) => Instant::now() < deadline,
    }
}

/// True while either latch holds an error, regardless of the backoff
/// window. Used by hibernation gating and status reporting; unreadable
/// project deps prove nothing and keep counting as active.
pub(crate) fn active(worker: &Worker) -> bool {
    if worker.inner.persistence_error.lock().unwrap().is_some() {
        return true;
    }
    match worker.inner.state.project.require() {
        Ok(deps) => deps.persistence_error.lock().unwrap().is_some(),
        Err(_) => true,
    }
}

/// A dispatch cycle that reconciled capacity, recovered brain/todo state
/// and processed every queue entry exercised journal, store and todo
/// persistence end to end: clear both latches and reset the schedule. A
/// genuinely still-broken store re-latches on its next failure.
pub(crate) fn recover(worker: &Worker) {
    if let Ok(deps) = worker.inner.state.project.require() {
        *deps.persistence_error.lock().unwrap() = None;
    }
    *worker.inner.persistence_error.lock().unwrap() = None;
    let mut retry = worker.inner.persistence_retry.lock().unwrap();
    retry.attempts = 0;
    retry.next_probe_at = None;
}
