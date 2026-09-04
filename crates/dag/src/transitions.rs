//! DAG run state machine — the `NodeTaskStatus` semantics transplanted to
//! workflow runs so claim / cancel-piggyback / lost-sweep reuse the proven
//! `node_tasks` protocols verbatim.
//!
//! Grid: `pending -> running | cancelling | cancelled | error`,
//! `running -> cancelling | done | cancelled | error`,
//! `cancelling -> cancelled | done | error`. Terminal states freeze.
//! `cancelling` is the collapse lane: a cancel request observed while the
//! node still works converges through the node's own abort path to
//! `cancelled` (or `done`/`error` if the run beat the request).

use serde::{Deserialize, Serialize};

/// Lifecycle status of one DAG run (stored as TEXT in `dag_runs.status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DagRunStatus {
    Pending,
    Running,
    Cancelling,
    Done,
    Error,
    Cancelled,
}

impl DagRunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            DagRunStatus::Pending => "pending",
            DagRunStatus::Running => "running",
            DagRunStatus::Cancelling => "cancelling",
            DagRunStatus::Done => "done",
            DagRunStatus::Error => "error",
            DagRunStatus::Cancelled => "cancelled",
        }
    }

    /// Parse the TEXT column value; unknown strings are impossible (schema
    /// only ever writes [`DagRunStatus::as_str`]) but callers still get a
    /// total function via `error`.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "pending" => DagRunStatus::Pending,
            "running" => DagRunStatus::Running,
            "cancelling" => DagRunStatus::Cancelling,
            "done" => DagRunStatus::Done,
            "error" => DagRunStatus::Error,
            "cancelled" => DagRunStatus::Cancelled,
            _ => return None,
        })
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            DagRunStatus::Done | DagRunStatus::Error | DagRunStatus::Cancelled
        )
    }
}

impl std::fmt::Display for DagRunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Terminal outcome of ONE step (event-projected; the server keeps no
/// per-step state — the UI folds `step_done` events).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    Done,
    Error,
    /// Step was aborted as part of a run-level cancel.
    Cancelled,
}

impl StepOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, StepOutcome::Done)
    }
}

/// The legal-transition grid (see module docs). Mirrors
/// `libsql_store/node_state.rs::transition_allowed` on purpose.
pub fn transition_allowed(from: DagRunStatus, to: DagRunStatus) -> bool {
    use DagRunStatus::{Cancelled, Cancelling, Done, Error, Pending, Running};
    matches!(
        (from, to),
        (Pending, Running)
            | (Pending, Cancelling)
            | (Pending, Cancelled)
            | (Pending, Error)
            | (Running, Cancelling)
            | (Running, Done)
            | (Running, Cancelled)
            | (Running, Error)
            | (Cancelling, Cancelled)
            | (Cancelling, Done)
            | (Cancelling, Error)
    )
}

#[cfg(test)]
mod tests {
    use super::{transition_allowed, DagRunStatus as S, StepOutcome};

    /// Same exhaustive-grid test shape as `node_state.rs`: every edge not on
    /// the legal list must be rejected, including the tempting
    /// `pending -> done` skip and any move out of a terminal state.
    #[test]
    fn transition_grid_rejects_illegal_moves_and_terminal_freeze() {
        let legal = [
            (S::Pending, S::Running),
            (S::Pending, S::Cancelling),
            (S::Pending, S::Cancelled),
            (S::Pending, S::Error),
            (S::Running, S::Cancelling),
            (S::Running, S::Done),
            (S::Running, S::Cancelled),
            (S::Running, S::Error),
            (S::Cancelling, S::Cancelled),
            (S::Cancelling, S::Done),
            (S::Cancelling, S::Error),
        ];
        let all = [
            S::Pending,
            S::Running,
            S::Done,
            S::Error,
            S::Cancelled,
            S::Cancelling,
        ];
        for from in all {
            for to in all {
                let expect = legal.contains(&(from, to));
                assert_eq!(transition_allowed(from, to), expect, "{from:?} -> {to:?}");
            }
        }
    }

    #[test]
    fn status_str_roundtrip_and_terminality() {
        for s in all_statuses() {
            assert_eq!(S::parse(s.as_str()), Some(s));
        }
        assert!(S::parse("nope").is_none());
        for t in [S::Done, S::Error, S::Cancelled] {
            assert!(t.is_terminal());
        }
        for live in [S::Pending, S::Running, S::Cancelling] {
            assert!(!live.is_terminal());
        }
        assert!(StepOutcome::Done.is_success());
        assert!(!StepOutcome::Error.is_success());
        assert!(!StepOutcome::Cancelled.is_success());
    }

    fn all_statuses() -> [S; 6] {
        [
            S::Pending,
            S::Running,
            S::Cancelling,
            S::Done,
            S::Error,
            S::Cancelled,
        ]
    }
}
