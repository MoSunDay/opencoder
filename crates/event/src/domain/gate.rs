//! Global capability gate.
//!
//! Pausing stops *dispatch* of new capability triggers; the durable event
//! log keeps accepting appends so nothing emitted during a pause is lost.
//! Resuming flips the gate back; consumers replay the log from their cursor.

use serde::{Deserialize, Serialize};

/// Durable pause state shared by every capability dispatcher.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerGate {
    pub paused: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Wall-clock milliseconds of the last transition (0 = never changed).
    #[serde(default)]
    pub changed_at_ms: i64,
}

/// Gate value after a pause transition.
pub fn paused(reason: impl Into<String>, now_ms: i64) -> SchedulerGate {
    SchedulerGate {
        paused: true,
        reason: Some(reason.into()),
        changed_at_ms: now_ms,
    }
}

/// Gate value after a resume transition.
pub fn resumed(now_ms: i64) -> SchedulerGate {
    SchedulerGate {
        paused: false,
        reason: None,
        changed_at_ms: now_ms,
    }
}

/// May a dispatcher run a capability trigger right now?
pub fn allows_dispatch(gate: &SchedulerGate) -> bool {
    !gate.paused
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_gate_dispatches() {
        let gate = SchedulerGate::default();
        assert!(allows_dispatch(&gate));
        assert_eq!(gate.reason, None);
    }

    #[test]
    fn pause_records_reason_and_blocks_dispatch() {
        let gate = paused("maintenance", 7);
        assert!(!allows_dispatch(&gate));
        assert_eq!(gate.reason.as_deref(), Some("maintenance"));
        assert_eq!(gate.changed_at_ms, 7);
    }

    #[test]
    fn resume_clears_reason_and_reallows_dispatch() {
        let gate = resumed(9);
        assert!(allows_dispatch(&gate));
        assert_eq!(gate.reason, None);
        assert_eq!(gate.changed_at_ms, 9);
    }
}

/// Pause transition: keeps the previous reason when no new one is given.
pub fn pause(gate: &SchedulerGate, reason: Option<String>, now_ms: i64) -> SchedulerGate {
    SchedulerGate {
        paused: true,
        reason: reason.or_else(|| gate.reason.clone()),
        changed_at_ms: now_ms,
    }
}

/// Resume transition: clears the pause reason.
pub fn resume(gate: &SchedulerGate, now_ms: i64) -> SchedulerGate {
    if !gate.paused {
        return gate.clone();
    }
    resumed(now_ms)
}

#[cfg(test)]
mod transition_tests {
    use super::*;

    #[test]
    fn pause_keeps_existing_reason_and_resume_is_idempotent() {
        let first = pause(&SchedulerGate::default(), Some("maintenance".into()), 1);
        let second = pause(&first, None, 2);
        assert_eq!(second.reason.as_deref(), Some("maintenance"));
        let open = resume(&second, 3);
        assert!(!open.paused);
        assert_eq!(resume(&open, 4), open);
    }
}
