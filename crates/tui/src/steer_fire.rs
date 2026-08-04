//! Shared steer-interrupt firing logic used by both the keyboard Enter path
//! (`KeyAction::Steer`) and the mouse `>` button (`MouseOutcome::SteerSubmit`).
//!
//! Extracted from `app.rs` so the fire logic is not duplicated and `app.rs`
//! stays within the line budget.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use opencoder_session::SharedCancel;
use tokio_util::sync::CancellationToken;

use crate::chat::ChatView;
use super::steer_dispatch;
use super::subagent_input;

/// Resolve the steer action and fire the appropriate interrupt.
///
/// Returns the resolved [`steer_dispatch::Action`] so the caller can handle
/// `StartTurn` (which requires async `start_turn` and mutable state not
/// available in this synchronous helper).
pub(crate) fn fire_steer_interrupt(
    subagent_focus: Option<usize>,
    running: bool,
    child_cancels: &Arc<Mutex<HashMap<String, CancellationToken>>>,
    child_turn_cancels: &Arc<Mutex<HashMap<String, SharedCancel>>>,
    turn_cancel: &SharedCancel,
    chat: &ChatView,
) -> steer_dispatch::Action {
    let sub_focused = subagent_focus.is_some();
    // fire_child_cancels both checks AND cancels children.
    let has_children = !sub_focused
        && running
        && opencoder_session::fire_child_cancels(child_cancels);
    let action = steer_dispatch::resolve(
        sub_focused,
        running,
        has_children,
        !chat.steer_items.is_empty(),
    );
    match action {
        steer_dispatch::Action::Subagent => {
            subagent_input::fire_subagent_turn_cancel(
                child_turn_cancels,
                chat,
                subagent_focus,
            );
        }
        steer_dispatch::Action::SteerParent => {
            opencoder_session::fire_turn_cancel(turn_cancel);
        }
        _ => {}
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tokio_util::sync::CancellationToken;

    fn fresh_cancel() -> SharedCancel {
        Arc::new(Mutex::new(CancellationToken::new()))
    }

    fn empty_cancels() -> Arc<Mutex<HashMap<String, CancellationToken>>> {
        Arc::new(Mutex::new(HashMap::new()))
    }

    fn empty_turn_cancels() -> Arc<Mutex<HashMap<String, SharedCancel>>> {
        Arc::new(Mutex::new(HashMap::new()))
    }

    // Core wiring test: a running parent with a pending steer must resolve to
    // SteerParent AND actually fire the shared turn_cancel token. This is the
    // G1 fix — the keyboard Enter path (`KeyAction::Steer`) and the `>` button
    // (`SteerSubmit`) both route through `fire_steer_interrupt`.
    #[test]
    fn running_parent_with_steer_fires_turn_cancel() {
        let turn_cancel = fresh_cancel();
        let child_cancels = empty_cancels();
        let child_turn_cancels = empty_turn_cancels();
        let mut chat = ChatView::default();
        chat.steer_items.push((1, "stop now".into()));

        let action = fire_steer_interrupt(
            None,
            true,
            &child_cancels,
            &child_turn_cancels,
            &turn_cancel,
            &chat,
        );

        assert_eq!(action, steer_dispatch::Action::SteerParent);
        assert!(
            turn_cancel.lock().unwrap().is_cancelled(),
            "running parent with a pending steer must fire the turn_cancel"
        );
    }

    // Idle parent (not running) resolves to StartTurn and must NOT fire the
    // turn_cancel — a fresh turn should start, not interrupt a nonexistent one.
    #[test]
    fn idle_parent_resolves_start_turn_without_firing() {
        let turn_cancel = fresh_cancel();
        let child_cancels = empty_cancels();
        let child_turn_cancels = empty_turn_cancels();
        let chat = ChatView::default();

        let action = fire_steer_interrupt(
            None,
            false,
            &child_cancels,
            &child_turn_cancels,
            &turn_cancel,
            &chat,
        );

        assert_eq!(action, steer_dispatch::Action::StartTurn);
        assert!(
            !turn_cancel.lock().unwrap().is_cancelled(),
            "idle path must not fire the turn_cancel"
        );
    }

    // Running parent with nothing pending is a Noop and must not fire.
    #[test]
    fn running_parent_with_nothing_pending_is_noop() {
        let turn_cancel = fresh_cancel();
        let child_cancels = empty_cancels();
        let child_turn_cancels = empty_turn_cancels();
        let chat = ChatView::default();

        let action = fire_steer_interrupt(
            None,
            true,
            &child_cancels,
            &child_turn_cancels,
            &turn_cancel,
            &chat,
        );

        assert_eq!(action, steer_dispatch::Action::Noop);
        assert!(
            !turn_cancel.lock().unwrap().is_cancelled(),
            "no-op path must not fire the turn_cancel"
        );
    }
}
