//! Pure node-task state-machine logic.
//!
//! Kept free of SQL so the legality grid is unit-testable without a database;
//! `nodes.rs` calls [`transition_allowed`] inside its update transaction.

use crate::types::NodeTaskStatus;

/// The legal-transition grid for the node task state machine:
/// - `pending -> running | cancelling | cancelled | error`
/// - `running -> cancelling | done | cancelled | error`
/// - `cancelling -> cancelled | done | error`
///
/// Terminal states (`done`/`error`/`cancelled`) freeze — every other edge is
/// rejected by `update_node_task_status`.
pub(crate) fn transition_allowed(from: NodeTaskStatus, to: NodeTaskStatus) -> bool {
    use NodeTaskStatus::{Cancelled, Cancelling, Done, Error, Pending, Running};
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
    use super::transition_allowed;
    use crate::types::NodeTaskStatus as S;

    /// Every illegal edge on the grid must be rejected — especially the
    /// tempting skip `pending -> done`: a task that never ran would leave its
    /// synthetic session un-executed, so collapse must go through cancelling.
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
        for (from, to) in legal {
            assert!(transition_allowed(from, to), "{from:?} -> {to:?} is legal");
        }
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
                if !legal.contains(&(from, to)) {
                    assert!(
                        !transition_allowed(from, to),
                        "{from:?} -> {to:?} must be illegal"
                    );
                }
            }
        }
    }

    #[test]
    fn only_done_error_cancelled_are_terminal() {
        assert!(!S::Pending.is_terminal());
        assert!(!S::Running.is_terminal());
        assert!(!S::Cancelling.is_terminal());
        for terminal in [S::Done, S::Error, S::Cancelled] {
            assert!(terminal.is_terminal(), "{terminal:?} is terminal");
            assert!(!transition_allowed(terminal, terminal));
        }
    }
}
