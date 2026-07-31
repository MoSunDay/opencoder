//! Pure decision logic for the `>` steer button (`MouseOutcome::SteerSubmit`).
//!
//! Extracted from `app.rs`'s inline if-else chain so the G1 invariant — "a
//! parent `>` with no running children STEERS via `fire_turn_cancel` rather
//! than hard-aborting via `cancel.cancel()`" — is unit-testable without
//! driving the full TUI event loop.
//!
//! Mirrors the `gate_compact` / `gate_clear_all` pattern in `worker.rs`:
//! the event loop feeds raw booleans into a pure resolver and `match`es on
//! the returned action.

/// Action the SteerSubmit handler should take.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// A child subagent row is focused — fire that child's turn-cancel.
    Subagent,
    /// The parent is running with live children — they were just cancelled so
    /// the parent absorbs the steer at the next turn boundary.
    CancelChildren,
    /// The parent is running with NO children but a steer is pending — steer
    /// the parent's own current turn via `fire_turn_cancel`. This is the G1
    /// fix: previously this path called `cancel.cancel()` (hard abort).
    SteerParent,
    /// Nothing is running — start a fresh empty-prompt turn.
    StartTurn,
    /// No children, nothing running, and no steer pending — no-op.
    Noop,
}

/// Resolve which action the `>` steer button should take.
///
/// * `subagent_focused` — a child subagent row is selected in the UI.
/// * `running` — the parent session has a turn in flight.
/// * `has_children` — at least one child cancellation token was registered
///   (and has now been fired by `fire_child_cancels`).
/// * `has_pending_steer` — `chat.steer_items` is non-empty (a steer row was
///   clicked and is waiting to be absorbed).
pub(crate) fn resolve(
    subagent_focused: bool,
    running: bool,
    has_children: bool,
    has_pending_steer: bool,
) -> Action {
    if subagent_focused {
        Action::Subagent
    } else if running {
        if has_children {
            Action::CancelChildren
        } else if has_pending_steer {
            Action::SteerParent
        } else {
            Action::Noop
        }
    } else {
        Action::StartTurn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // G1 regression guard: a parent `>` with no running children but a pending
    // steer must resolve to SteerParent (fire_turn_cancel), NOT a hard abort.
    // Before the fix this path called cancel.cancel() which killed the run_loop.
    #[test]
    fn running_parent_with_pending_steer_steers_not_aborts() {
        assert_eq!(
            resolve(false, true, false, true),
            Action::SteerParent,
            "parent > with pending steer must steer, not hard-abort"
        );
    }

    // G1 guard: no children and no pending steer → Noop (must NOT abort).
    #[test]
    fn running_parent_with_nothing_to_do_is_noop() {
        assert_eq!(
            resolve(false, true, false, false),
            Action::Noop,
            "parent > with nothing pending must be a no-op, not an abort"
        );
    }

    #[test]
    fn running_parent_with_children_cancels_children() {
        assert_eq!(resolve(false, true, true, true), Action::CancelChildren);
        assert_eq!(resolve(false, true, true, false), Action::CancelChildren);
    }

    #[test]
    fn subagent_focused_always_targets_subagent() {
        assert_eq!(resolve(true, true, true, true), Action::Subagent);
        assert_eq!(resolve(true, true, false, false), Action::Subagent);
        assert_eq!(resolve(true, false, false, false), Action::Subagent);
    }

    #[test]
    fn idle_parent_starts_new_turn() {
        assert_eq!(resolve(false, false, false, false), Action::StartTurn);
        assert_eq!(resolve(false, false, true, true), Action::StartTurn);
    }
}
