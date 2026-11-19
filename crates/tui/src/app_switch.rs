//! Atomic session-switch intent.
//!
//! A switch has two halves that must move together: the [`TaskPick`] the TUI
//! is switching to, and the parent session to adopt as `main_task_id` once
//! that switch succeeds. Keeping both in one [`PendingSwitch`] means a late
//! `/agent` capability query can never replace a `/subtasks` Enter descend,
//! and a failed switch can never leave a stale parent arm behind.

use crate::remote::Selection;
use crate::task::TaskPick;

/// One staged session switch: the pick plus the parent to adopt afterwards.
pub(crate) struct PendingSwitch {
    pub pick: TaskPick,
    pub parent_after_switch: Option<String>,
}

/// A plain switch: adopts no parent (clears any current descent pointer on
/// success).
pub(crate) fn plain(pick: TaskPick) -> PendingSwitch {
    PendingSwitch {
        pick,
        parent_after_switch: None,
    }
}

/// A descent switch: adopts `parent` as `main_task_id` once the switch lands.
pub(crate) fn descend(pick: TaskPick, parent: String) -> PendingSwitch {
    PendingSwitch {
        pick,
        parent_after_switch: Some(parent),
    }
}

/// Stage `next`, dropping any in-flight `/agent` capability query first so
/// its late result cannot overwrite the staged intent.
pub(crate) fn stage(
    pending: &mut Option<PendingSwitch>,
    selection: &mut Selection,
    next: PendingSwitch,
) {
    *selection = None;
    *pending = Some(next);
}

/// Accept a polled `/agent` pick only when nothing is staged: a descend (or
/// any other explicit switch) always wins over a late capability query.
pub(crate) fn accept_polled(pending: &mut Option<PendingSwitch>, pick: TaskPick) {
    if pending.is_none() {
        *pending = Some(plain(pick));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live_selection() -> (
        tokio::sync::oneshot::Sender<anyhow::Result<TaskPick>>,
        Selection,
    ) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        (tx, Some(rx))
    }

    /// Staging a descend drops the in-flight `/agent` receiver and records
    /// both halves of the intent.
    #[test]
    fn staging_descend_cancels_inflight_selection() {
        let (_tx, mut selection) = live_selection();
        let mut pending = None;
        stage(
            &mut pending,
            &mut selection,
            descend(TaskPick::Resume("child".into()), "parent".into()),
        );
        assert!(selection.is_none(), "in-flight query receiver is dropped");
        let staged = pending.expect("descend staged");
        assert!(matches!(staged.pick, TaskPick::Resume(id) if id == "child"));
        assert_eq!(staged.parent_after_switch.as_deref(), Some("parent"));
    }

    /// A poll result arriving after a descend is staged is discarded.
    #[test]
    fn late_poll_cannot_replace_pending_descend() {
        let mut pending = Some(descend(TaskPick::Resume("child".into()), "parent".into()));
        accept_polled(&mut pending, TaskPick::New);
        let staged = pending.expect("staged descend survives the late poll");
        assert!(matches!(staged.pick, TaskPick::Resume(id) if id == "child"));
        assert_eq!(staged.parent_after_switch.as_deref(), Some("parent"));
    }

    /// Plain switches carry no parent, so success clears the descent pointer.
    #[test]
    fn plain_switch_has_no_parent() {
        let (_tx, mut selection) = live_selection();
        let mut pending = None;
        stage(&mut pending, &mut selection, plain(TaskPick::New));
        assert!(selection.is_none());
        let staged = pending.expect("plain switch staged");
        assert!(matches!(staged.pick, TaskPick::New));
        assert!(staged.parent_after_switch.is_none());
    }

    /// With nothing staged, a polled pick is accepted as a plain switch.
    #[test]
    fn poll_fills_empty_pending_as_plain() {
        let mut pending = None;
        accept_polled(&mut pending, TaskPick::Fork("forked".into()));
        let staged = pending.expect("poll result staged");
        assert!(matches!(staged.pick, TaskPick::Fork(id) if id == "forked"));
        assert!(staged.parent_after_switch.is_none());
    }
}
