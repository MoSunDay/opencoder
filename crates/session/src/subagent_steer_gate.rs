//! Runtime admission gate for non-interrupting subagent steers.
//!
//! A child runner may only finish after it has atomically observed that no
//! steer admission is in flight and no committed steer arrived since the
//! current provider-turn boundary. This closes the gap between the final
//! store poll and `Done`, where an accepted steer used to become stranded.

use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

#[derive(Debug, Default)]
struct GateState {
    epoch: u64,
    reservations: usize,
    closed: bool,
}

/// Decision returned at a child runner's final idle boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdleDecision {
    /// A steer committed after the boundary snapshot; start another loop.
    Continue,
    /// No admission can still commit; the child may finish safely.
    Close,
}

/// Coordinates external steer admission with one subagent runner.
#[derive(Debug, Default)]
pub struct SubagentSteerGate {
    state: Mutex<GateState>,
    changed: Notify,
}

impl SubagentSteerGate {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Reserve one admission before starting the asynchronous store write.
    /// Returns `None` once the child has crossed its terminal boundary.
    pub fn reserve(self: &Arc<Self>) -> Option<SteerReservation> {
        let mut state = self.state.lock().ok()?;
        if state.closed {
            return None;
        }
        state.reservations += 1;
        drop(state);
        Some(SteerReservation {
            gate: self.clone(),
            active: true,
        })
    }

    /// Snapshot the committed-admission generation before polling the store.
    pub fn epoch(&self) -> u64 {
        self.state.lock().map(|state| state.epoch).unwrap_or(0)
    }

    /// Settle the final idle boundary without racing an asynchronous admit.
    pub async fn settle_idle(&self, observed_epoch: u64) -> IdleDecision {
        loop {
            // Register before inspecting state so a reservation completion
            // between the inspection and await cannot be missed. Even if a
            // notification lands before this waiter's first poll, `notify_one`
            // stores a permit (unlike `notify_waiters`), so the await below
            // completes and the loop re-inspects the changed state instead of
            // hanging on a lost wakeup.
            let changed = self.changed.notified();
            let decision = match self.state.lock() {
                Ok(mut state) => {
                    if state.closed {
                        Some(IdleDecision::Close)
                    } else if state.epoch != observed_epoch {
                        Some(IdleDecision::Continue)
                    } else if state.reservations == 0 {
                        state.closed = true;
                        Some(IdleDecision::Close)
                    } else {
                        None
                    }
                }
                // A poisoned gate cannot safely admit more work. Closing is
                // the conservative terminal action; callers will reject new
                // reservations because locking also fails.
                Err(_) => Some(IdleDecision::Close),
            };
            if let Some(decision) = decision {
                return decision;
            }
            changed.await;
        }
    }

    /// Stop accepting steers on hard-cancel/forced-cleanup paths.
    pub fn force_close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
        }
        self.changed.notify_one();
    }

    #[cfg(test)]
    fn is_closed(&self) -> bool {
        self.state.lock().map(|state| state.closed).unwrap_or(true)
    }
}

/// In-flight admission permit. Dropping it aborts the reservation; committing
/// it publishes a new epoch only if the child is still accepting work.
pub struct SteerReservation {
    gate: Arc<SubagentSteerGate>,
    active: bool,
}

impl SteerReservation {
    /// Publish the completed store write. `false` means a force-close won the
    /// race; the caller must delete the just-written input before returning.
    pub fn commit(mut self) -> bool {
        self.active = false;
        let accepted = match self.gate.state.lock() {
            Ok(mut state) => {
                state.reservations = state.reservations.saturating_sub(1);
                if state.closed {
                    false
                } else {
                    state.epoch = state.epoch.saturating_add(1);
                    true
                }
            }
            Err(_) => false,
        };
        self.gate.changed.notify_one();
        accepted
    }
}

impl Drop for SteerReservation {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        if let Ok(mut state) = self.gate.state.lock() {
            state.reservations = state.reservations.saturating_sub(1);
        }
        self.gate.changed.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn committed_admission_forces_another_boundary() {
        let gate = SubagentSteerGate::new();
        let observed = gate.epoch();
        assert!(gate.reserve().unwrap().commit());
        assert_eq!(gate.settle_idle(observed).await, IdleDecision::Continue);
        assert!(!gate.is_closed());
    }

    #[tokio::test]
    async fn idle_waits_for_an_in_flight_admission() {
        let gate = SubagentSteerGate::new();
        let observed = gate.epoch();
        let reservation = gate.reserve().unwrap();
        let waiter = {
            let gate = gate.clone();
            tokio::spawn(async move { gate.settle_idle(observed).await })
        };
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());
        assert!(reservation.commit());
        assert_eq!(waiter.await.unwrap(), IdleDecision::Continue);
    }

    #[tokio::test]
    async fn close_rejects_late_admission() {
        let gate = SubagentSteerGate::new();
        assert_eq!(gate.settle_idle(gate.epoch()).await, IdleDecision::Close);
        assert!(gate.is_closed());
        assert!(gate.reserve().is_none());
    }

    #[test]
    fn force_close_makes_existing_reservation_fail_commit() {
        let gate = SubagentSteerGate::new();
        let reservation = gate.reserve().unwrap();
        gate.force_close();
        assert!(!reservation.commit());
        assert!(gate.reserve().is_none());
    }

    /// Regression: a notification delivered while NO waiter is registered must
    /// not be lost. `settle_idle` registers its waiter only on first poll, so
    /// an admission that commits in the window before that poll relies on the
    /// stored permit (notify_one) to wake it. With notify_waiters the wake is
    /// dropped and the child would hang on its terminal boundary.
    #[tokio::test]
    async fn committed_before_any_waiter_is_not_lost() {
        let gate = SubagentSteerGate::new();
        let observed = gate.epoch();
        // Commit BEFORE any settle_idle has registered a waiter.
        assert!(gate.reserve().unwrap().commit());
        // A late settle_idle must still observe the bump, not block forever.
        let decision = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            gate.settle_idle(observed),
        )
        .await
        .expect("settle_idle must not hang on a pre-registration commit");
        assert_eq!(decision, IdleDecision::Continue);
    }

    /// Regression: force_close racing a waiting settle_idle must wake it (the
    /// waiter re-checks state and closes). Guards against losing the wake on
    /// the forced-cleanup path.
    #[tokio::test]
    async fn force_close_wakes_an_already_waiting_settle_idle() {
        let gate = SubagentSteerGate::new();
        let observed = gate.epoch();
        let reservation = gate.reserve().unwrap();
        let waiter = {
            let gate = gate.clone();
            tokio::spawn(async move { gate.settle_idle(observed).await })
        };
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());
        gate.force_close();
        // The waiter must wake and close promptly (not block until drop).
        let decision = tokio::time::timeout(std::time::Duration::from_millis(500), waiter)
            .await
            .expect("settle_idle must wake on force_close")
            .unwrap();
        assert_eq!(decision, IdleDecision::Close);
        // The in-flight reservation also fails commit after the close.
        assert!(!reservation.commit());
    }
}
