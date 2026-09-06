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
        None => status,
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
}
