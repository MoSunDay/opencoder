use opencoder_core::fleet::ExecutionStatus;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StopIntent {
    Cancel,
    Interrupt,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct Lifecycle {
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub todo_reruns: std::collections::BTreeMap<String, crate::operations::todo::Control>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_intent: Option<StopIntent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub todo_initialization: Option<TodoInitialization>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TodoInitialization {
    Accepted,
    Initialized,
}

impl Lifecycle {
    pub(crate) fn accepted_todo() -> Self {
        Self {
            todo_initialization: Some(TodoInitialization::Accepted),
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Transition {
    pub status: ExecutionStatus,
    pub intent: Option<StopIntent>,
}

pub(crate) fn request_stop(
    status: ExecutionStatus,
    existing: Option<StopIntent>,
    requested: StopIntent,
    active: bool,
) -> Transition {
    if status.terminal() {
        return Transition {
            status,
            intent: existing,
        };
    }
    let merged = merge_intent(existing, requested);
    let intent = Some(merged);
    let status = if active {
        ExecutionStatus::Cancelling
    } else {
        stopped_status(merged)
    };
    Transition { status, intent }
}

pub(crate) fn recover(status: ExecutionStatus, intent: Option<StopIntent>) -> Transition {
    if status.terminal() {
        return Transition { status, intent };
    }
    let status = match intent {
        Some(StopIntent::Cancel) => ExecutionStatus::Cancelled,
        Some(StopIntent::Interrupt) => ExecutionStatus::Interrupted,
        None if matches!(
            status,
            ExecutionStatus::Pending | ExecutionStatus::Running | ExecutionStatus::Cancelling
        ) =>
        {
            ExecutionStatus::Interrupted
        }
        // A paused run stays paused across recovery: only an explicit resume
        // may return it to scheduling.
        None => status,
    };
    Transition { status, intent }
}

/// A non-terminal Brain control command. Pausing freezes the root's future
/// scheduling (started children and their receipts keep flowing); resuming
/// returns a paused root to idle so the normal wake flow re-enqueues it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BrainControl {
    Pause,
    Resume,
}

/// Apply a Brain control command to the journal status. Terminal runs are
/// immutable, and the durable result, error and stop intent are never
/// touched: a pause must never look like a stop or a finish.
pub(crate) fn brain_control(
    status: ExecutionStatus,
    intent: Option<StopIntent>,
    action: BrainControl,
) -> Transition {
    if status.terminal() {
        return Transition { status, intent };
    }
    let status = match action {
        BrainControl::Pause => ExecutionStatus::Paused,
        BrainControl::Resume => ExecutionStatus::Idle,
    };
    Transition { status, intent }
}

pub(crate) fn finalize(
    current: ExecutionStatus,
    intent: Option<StopIntent>,
    proposed: ExecutionStatus,
) -> Transition {
    if current.terminal() {
        return Transition {
            status: current,
            intent,
        };
    }
    let status = intent.map(stopped_status).unwrap_or(proposed);
    Transition { status, intent }
}

pub(crate) fn can_start(status: ExecutionStatus) -> bool {
    matches!(
        status,
        ExecutionStatus::Pending
            | ExecutionStatus::Idle
            | ExecutionStatus::Interrupted
            | ExecutionStatus::Error
    )
}

pub(crate) fn can_launch(status: ExecutionStatus, resume: bool) -> bool {
    if resume {
        matches!(
            status,
            ExecutionStatus::Idle | ExecutionStatus::Interrupted | ExecutionStatus::Error
        )
    } else {
        status == ExecutionStatus::Pending
    }
}

/// A paused execution is deliberately excluded: pausing survives a restart
/// without interruption, and only the explicit resume command (which returns
/// the record to idle) re-enters the launch paths.
pub(crate) fn shutdown_interrupt_required(status: ExecutionStatus) -> bool {
    matches!(
        status,
        ExecutionStatus::Pending
            | ExecutionStatus::Running
            | ExecutionStatus::Idle
            | ExecutionStatus::Cancelling
    )
}

fn merge_intent(existing: Option<StopIntent>, requested: StopIntent) -> StopIntent {
    if existing == Some(StopIntent::Cancel) || requested == StopIntent::Cancel {
        StopIntent::Cancel
    } else {
        StopIntent::Interrupt
    }
}

fn stopped_status(intent: StopIntent) -> ExecutionStatus {
    match intent {
        StopIntent::Cancel => ExecutionStatus::Cancelled,
        StopIntent::Interrupt => ExecutionStatus::Interrupted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_is_continuable_but_can_still_be_cancelled() {
        assert!(can_start(ExecutionStatus::Idle));
        assert!(can_start(ExecutionStatus::Error));
        assert!(!can_start(ExecutionStatus::Done));
        assert!(!can_start(ExecutionStatus::Cancelled));
        assert!(shutdown_interrupt_required(ExecutionStatus::Idle));
        assert!(!shutdown_interrupt_required(ExecutionStatus::Error));
        assert!(can_launch(ExecutionStatus::Pending, false));
        assert!(!can_launch(ExecutionStatus::Interrupted, false));
        assert!(can_launch(ExecutionStatus::Interrupted, true));
        assert_eq!(
            request_stop(ExecutionStatus::Idle, None, StopIntent::Cancel, false).status,
            ExecutionStatus::Cancelled
        );
    }

    #[test]
    fn finish_and_stop_race_has_one_deterministic_winner() {
        let finished = finalize(ExecutionStatus::Running, None, ExecutionStatus::Done);
        assert_eq!(
            request_stop(finished.status, finished.intent, StopIntent::Cancel, false).status,
            ExecutionStatus::Done
        );

        let stopping = request_stop(ExecutionStatus::Running, None, StopIntent::Cancel, true);
        assert_eq!(
            finalize(stopping.status, stopping.intent, ExecutionStatus::Done).status,
            ExecutionStatus::Cancelled
        );
    }

    #[test]
    fn cancel_dominates_interrupt_and_crash_recovery() {
        let interrupt = request_stop(ExecutionStatus::Running, None, StopIntent::Interrupt, true);
        let cancel = request_stop(interrupt.status, interrupt.intent, StopIntent::Cancel, true);
        let late_interrupt =
            request_stop(cancel.status, cancel.intent, StopIntent::Interrupt, true);
        assert_eq!(late_interrupt.intent, Some(StopIntent::Cancel));
        assert_eq!(
            recover(late_interrupt.status, late_interrupt.intent).status,
            ExecutionStatus::Cancelled
        );
    }

    #[test]
    fn legacy_unfinished_work_recovers_as_interrupted() {
        for status in [
            ExecutionStatus::Pending,
            ExecutionStatus::Running,
            ExecutionStatus::Cancelling,
        ] {
            assert_eq!(recover(status, None).status, ExecutionStatus::Interrupted);
        }
    }

    #[test]
    fn brain_control_pauses_and_resumes_without_touching_stop_semantics() {
        let paused = brain_control(
            ExecutionStatus::Idle,
            Some(StopIntent::Interrupt),
            BrainControl::Pause,
        );
        assert_eq!(paused.status, ExecutionStatus::Paused);
        // The stop intent and the terminal result semantics stay untouched.
        assert_eq!(paused.intent, Some(StopIntent::Interrupt));
        assert!(!paused.status.terminal());

        let resumed = brain_control(paused.status, paused.intent, BrainControl::Resume);
        assert_eq!(resumed.status, ExecutionStatus::Idle);
        assert_eq!(resumed.intent, Some(StopIntent::Interrupt));

        // A paused record also survives a running activation finalizing it.
        assert_eq!(
            finalize(ExecutionStatus::Paused, None, ExecutionStatus::Idle).status,
            ExecutionStatus::Idle
        );
        assert_eq!(
            finalize(ExecutionStatus::Paused, None, ExecutionStatus::Paused).status,
            ExecutionStatus::Paused
        );
        for status in [
            ExecutionStatus::Done,
            ExecutionStatus::Error,
            ExecutionStatus::Cancelled,
        ] {
            assert_eq!(
                brain_control(status, None, BrainControl::Pause).status,
                status,
                "terminal runs never move"
            );
            assert_eq!(
                brain_control(status, None, BrainControl::Resume).status,
                status,
                "terminal runs never move"
            );
        }
    }

    #[test]
    fn paused_runs_resume_explicitly_and_survive_recovery() {
        assert!(!ExecutionStatus::Paused.terminal());
        assert_eq!(ExecutionStatus::Paused.as_str(), "paused");
        // Pausing is not a shutdown interruption and recovery keeps it paused.
        assert!(!shutdown_interrupt_required(ExecutionStatus::Paused));
        assert_eq!(
            recover(ExecutionStatus::Paused, None).status,
            ExecutionStatus::Paused
        );
        // Nothing but the explicit control command re-enters scheduling.
        assert!(!can_start(ExecutionStatus::Paused));
        assert!(!can_launch(ExecutionStatus::Paused, true));
        assert_eq!(
            brain_control(ExecutionStatus::Paused, None, BrainControl::Resume).status,
            ExecutionStatus::Idle
        );
        assert!(can_launch(ExecutionStatus::Idle, true));
    }
}
