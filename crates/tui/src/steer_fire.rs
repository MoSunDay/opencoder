//! Steer dispatch: the interrupt route for the `>` mouse button.
//!
//! The keyboard Enter path (`KeyAction::Steer`) no longer lives here — it
//! moved to `steer_admit::submit_steer`, an off-loop optimistic actor that
//! persists + pushes onto the pending steer panel WITHOUT interrupting the
//! running turn (structurally interrupt-free: no turn_cancel anywhere in
//! that path; a stranded row is restarted by idle_rekick like any admit).
//!
//! - **Mouse `>` button** (`MouseOutcome::SteerSubmit`) ->
//!   [`fire_steer_interrupt`]: `steer_dispatch::resolve` + `fire_turn_cancel`,
//!   immediately interrupting the running turn.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use opencoder_session::SharedCancel;
use tokio_util::sync::CancellationToken;

use super::steer_dispatch;
use super::subagent_input;
use crate::chat::ChatView;

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
    // Liveness-aware focus: only a LIVE (`done == false`) subagent block
    // routes `>` to the child. A done subagent or a stale block index must
    // fall through to the PARENT path — the queue panel already shows the
    // parent's steer rows in that state (`app_display::steer_queue_sources`),
    // so the click must interrupt the parent and submit the steer instead of
    // silently no-oping inside `fire_subagent_turn_cancel`.
    let sub_focused = subagent_input::is_live_subagent_focus(chat, subagent_focus);
    // fire_child_cancels both checks AND cancels children. While a running
    // subagent is focused the `>` targets the CHILD's own turn token, so the
    // siblings are left untouched (no cascade).
    let has_children =
        !sub_focused && running && opencoder_session::fire_child_cancels(child_cancels);
    let action = steer_dispatch::resolve(
        sub_focused,
        running,
        has_children,
        !chat.steer_items.is_empty(),
    );
    match action {
        steer_dispatch::Action::Subagent => {
            subagent_input::fire_subagent_turn_cancel(child_turn_cancels, chat, subagent_focus);
        }
        steer_dispatch::Action::SteerParent | steer_dispatch::Action::CancelChildrenAndSteer => {
            opencoder_session::fire_turn_cancel(turn_cancel);
        }
        _ => {}
    }
    action
}

/// Outcome of a `>` button submit: the turn token was fired (steer admitted,
/// turn interrupted) or `StartTurn` — nothing pending, so the caller should
/// start a fresh turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SteerSubmitOutcome {
    StartTurn,
    SteerOnly,
}

/// `>` button submit path: resolve + fire interrupts, and tell the caller
/// whether a fresh turn must be started. Kept here (rather than in `app.rs`)
/// so the sync interrupt logic and its outcome stay next to
/// [`fire_steer_interrupt`].
pub(crate) fn handle_steer_submit(
    subagent_focus: Option<usize>,
    running: bool,
    child_cancels: &Arc<Mutex<HashMap<String, CancellationToken>>>,
    child_turn_cancels: &Arc<Mutex<HashMap<String, SharedCancel>>>,
    turn_cancel: &SharedCancel,
    chat: &ChatView,
) -> SteerSubmitOutcome {
    let action = fire_steer_interrupt(
        subagent_focus,
        running,
        child_cancels,
        child_turn_cancels,
        turn_cancel,
        chat,
    );
    match action {
        steer_dispatch::Action::StartTurn => SteerSubmitOutcome::StartTurn,
        _ => SteerSubmitOutcome::SteerOnly,
    }
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
    // SteerParent AND actually fire the shared turn_cancel token. The `>` button
    // (`SteerSubmit`) routes through `fire_steer_interrupt`; the keyboard Enter
    // path (`KeyAction::Steer`) does NOT (it admits without interrupting).
    #[test]
    fn running_parent_with_steer_fires_turn_cancel() {
        let turn_cancel = fresh_cancel();
        let child_cancels = empty_cancels();
        let mut chat = ChatView::default();
        chat.steer_items.push((1, "stop now".into()));

        let action = fire_steer_interrupt(
            None,
            true,
            &child_cancels,
            &empty_turn_cancels(),
            &turn_cancel,
            &chat,
        );

        assert_eq!(action, steer_dispatch::Action::SteerParent);
        assert!(
            turn_cancel.lock().unwrap().is_cancelled(),
            "running parent with a pending steer must fire the turn_cancel"
        );
    }

    // Stale-focus guard (G3): a focused subagent that is DONE must NOT swallow
    // the click. The panel shows the parent's steer rows in that state
    // (`app_display::steer_queue_sources`), so `>` must take the parent path
    // and fire turn_cancel — before this fix the click silently no-oped
    // inside `fire_subagent_turn_cancel` (no interrupt, no submit).
    #[test]
    fn done_subagent_focus_falls_back_to_parent_steer() {
        let turn_cancel = fresh_cancel();
        let mut chat = ChatView::default();
        chat.blocks.push(crate::chat::ChatBlock::Subagent {
            id: "task-1".into(),
            child_session_id: "sub-1".into(),
            kind: "explore".into(),
            prompt: "p".into(),
            view: ChatView::default(),
            done: true,
            ok: true,
            cancelled: false,
            summary: String::new(),
            started_at_ms: 0,
            elapsed_ms: None,
        });
        chat.steer_items.push((1, "stop now".into()));

        let action = fire_steer_interrupt(
            Some(0),
            true,
            &empty_cancels(),
            &empty_turn_cancels(),
            &turn_cancel,
            &chat,
        );

        assert_eq!(action, steer_dispatch::Action::SteerParent);
        assert!(
            turn_cancel.lock().unwrap().is_cancelled(),
            "done-subagent focus must fall back to the parent interrupt"
        );
    }

    // G3 companion: a stale focus index (block replaced/shifted since the
    // click target was registered) must likewise fall back to the parent path.
    #[test]
    fn stale_focus_index_falls_back_to_parent_steer() {
        let turn_cancel = fresh_cancel();
        let mut chat = ChatView::default();
        chat.steer_items.push((1, "stop now".into()));

        let action = fire_steer_interrupt(
            Some(9),
            true,
            &empty_cancels(),
            &empty_turn_cancels(),
            &turn_cancel,
            &chat,
        );

        assert_eq!(action, steer_dispatch::Action::SteerParent);
        assert!(turn_cancel.lock().unwrap().is_cancelled());
    }

    // Live-subagent focus still targets ONLY the child's turn token: the
    // parent's turn_cancel stays intact (the child absorbs its own steer).
    #[test]
    fn live_subagent_focus_targets_child_token_only() {
        let turn_cancel = fresh_cancel();
        let child_token: SharedCancel = fresh_cancel();
        let child_turn_cancels = empty_turn_cancels();
        child_turn_cancels
            .lock()
            .unwrap()
            .insert("task-1".into(), child_token.clone());
        let mut chat = ChatView::default();
        chat.blocks.push(crate::chat::ChatBlock::Subagent {
            id: "task-1".into(),
            child_session_id: "sub-1".into(),
            kind: "explore".into(),
            prompt: "p".into(),
            view: ChatView::default(),
            done: false,
            ok: false,
            cancelled: false,
            summary: String::new(),
            started_at_ms: 0,
            elapsed_ms: None,
        });
        chat.steer_items.push((1, "parent steer".into()));

        let action = fire_steer_interrupt(
            Some(0),
            true,
            &empty_cancels(),
            &child_turn_cancels,
            &turn_cancel,
            &chat,
        );

        assert_eq!(action, steer_dispatch::Action::Subagent);
        assert!(
            child_token.lock().unwrap().is_cancelled(),
            "live child focus must fire the child's turn token"
        );
        assert!(
            !turn_cancel.lock().unwrap().is_cancelled(),
            "live child focus must NOT interrupt the parent's turn"
        );
    }

    // G2 guard: a running parent with live children AND a pending steer must
    // fire turn_cancel (interrupt the parent turn) so the steer is absorbed in
    // one click. Previously this path only cancelled children and the user had to
    // click `>` a second time.
    #[test]
    fn running_parent_with_children_and_steer_fires_turn_cancel() {
        let turn_cancel = fresh_cancel();
        let child_cancels: Arc<Mutex<HashMap<String, CancellationToken>>> =
            Arc::new(Mutex::new(HashMap::new()));
        child_cancels
            .lock()
            .unwrap()
            .insert("child-1".into(), CancellationToken::new());
        let mut chat = ChatView::default();
        chat.steer_items.push((1, "stop now".into()));

        let action = fire_steer_interrupt(
            None,
            true,
            &child_cancels,
            &empty_turn_cancels(),
            &turn_cancel,
            &chat,
        );

        assert_eq!(action, steer_dispatch::Action::CancelChildrenAndSteer);
        assert!(
            turn_cancel.lock().unwrap().is_cancelled(),
            "parent > with children + pending steer must fire turn_cancel"
        );
    }

    // Idle parent (not running) resolves to StartTurn and must NOT fire the
    // turn_cancel — a fresh turn should start, not interrupt a nonexistent one.
    #[test]
    fn idle_parent_resolves_start_turn_without_firing() {
        let turn_cancel = fresh_cancel();
        let child_cancels = empty_cancels();
        let chat = ChatView::default();

        let action = fire_steer_interrupt(
            None,
            false,
            &child_cancels,
            &empty_turn_cancels(),
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
        let chat = ChatView::default();

        let action = fire_steer_interrupt(
            None,
            true,
            &child_cancels,
            &empty_turn_cancels(),
            &turn_cancel,
            &chat,
        );

        assert_eq!(action, steer_dispatch::Action::Noop);
        assert!(
            !turn_cancel.lock().unwrap().is_cancelled(),
            "no-op path must not fire the turn_cancel"
        );
    }

    // Subagent steer: while a running subagent is focused, `>` must fire ONLY
    // that child's own turn token — the parent turn_cancel and the sibling
    // hard-cancels stay untouched.
    #[test]
    fn focused_running_subagent_fires_only_its_own_turn_token() {
        let parent_turn = fresh_cancel();
        let child_turn = fresh_cancel();
        let child_cancels = empty_cancels();
        let child_turn_cancels: Arc<Mutex<HashMap<String, SharedCancel>>> =
            Arc::new(Mutex::new(HashMap::new()));
        child_turn_cancels
            .lock()
            .unwrap()
            .insert("child-1".into(), child_turn.clone());

        let mut chat = ChatView::default();
        chat.blocks.push(crate::chat::ChatBlock::Subagent {
            id: "child-1".into(),
            child_session_id: "sub-s".into(),
            kind: "explore".into(),
            prompt: "investigate".into(),
            view: ChatView::default(),
            done: false,
            ok: false,
            cancelled: false,
            summary: String::new(),
            started_at_ms: 0,
            elapsed_ms: None,
        });

        let action = fire_steer_interrupt(
            Some(0),
            true,
            &child_cancels,
            &child_turn_cancels,
            &parent_turn,
            &chat,
        );

        assert_eq!(action, steer_dispatch::Action::Subagent);
        assert!(
            child_turn.lock().unwrap().is_cancelled(),
            "focused subagent's own turn token must fire"
        );
        assert!(
            !parent_turn.lock().unwrap().is_cancelled(),
            "parent turn_cancel must NOT fire while a subagent is focused"
        );
    }

    // Architectural divergence guard: the keyboard Enter path
    // (`KeyAction::Steer`) and the `>` button path (`SteerSubmit`) MUST behave
    // differently when a turn is running with a pending steer.
    //
    //   - `>` button  -> resolve(...) -> SteerParent -> fire_turn_cancel (interrupt)
    //   - Enter key   -> admit only, never call resolve()/fire_steer_interrupt
    //
    // The keyboard path lets the running turn finish naturally; the admitted
    // steer is absorbed at the next idle/turn boundary by the runner (see
    // session `claim_steers` / late-steer peek). This test pins the `>` button
    // resolver so a regression that re-couples Enter to the interrupt path is
    // caught here.
    #[test]
    fn only_button_path_interrupts_running_turn_with_steer() {
        // `>` button: running parent, pending steer, no children -> interrupt.
        assert_eq!(
            steer_dispatch::resolve(false, true, false, true),
            steer_dispatch::Action::SteerParent,
            "`>` button must resolve to SteerParent (interrupt) when running"
        );
    }

    #[test]
    fn idle_steer_does_not_interrupt() {
        // Neither path fires an interrupt when nothing is running: the `>`
        // button resolves to StartTurn, and the keyboard Enter path would
        // simply admit (a fresh turn is started by the Submit path instead).
        assert_eq!(
            steer_dispatch::resolve(false, false, false, true),
            steer_dispatch::Action::StartTurn,
            "idle `>` with a pending steer must start a turn, not interrupt"
        );
    }
}
