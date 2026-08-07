//! Display-state helpers extracted from `app.rs` to keep the event loop concise.

use crate::chat::{ChatBlock, ChatView};

/// Decide which steer/queue sources the queue panel should show.
///
/// When a *running* subagent is focused we show its child view's steer items;
/// otherwise we fall back to the parent's steer and queue lists.
#[allow(clippy::type_complexity)]
pub(super) fn steer_queue_sources<'a>(
    chat: &'a ChatView,
    subagent_focus: Option<usize>,
    queue_items: &'a [(i64, String)],
) -> (&'a [(i64, String)], &'a [(i64, String)]) {
    if let Some(idx) = subagent_focus {
        match chat.blocks.get(idx) {
            Some(ChatBlock::Subagent {
                view, done: false, ..
            }) => (&view.steer_items, &[][..]),
            _ => (&chat.steer_items, queue_items),
        }
    } else {
        (&chat.steer_items, queue_items)
    }
}

/// Input is disabled only when a DONE subagent is focused (not when a running
/// one is — the user can still steer it).
pub(super) fn is_input_disabled(chat: &ChatView, subagent_focus: Option<usize>) -> bool {
    subagent_focus.is_some_and(|idx| {
        chat.blocks
            .get(idx)
            .is_none_or(|b| matches!(b, ChatBlock::Subagent { done: true, .. }))
    })
}

/// Turn-duration timer value shown at the tail of the body content.
///
/// - Running subagent focused → its live elapsed (`now - started_at_ms`).
/// - No subagent focused → the top-level run accumulator.
/// - Done/finished subagent focused, or invalid index → 0.
pub(super) fn display_turn_ms(
    chat: &ChatView,
    subagent_focus: Option<usize>,
    run_elapsed_ms: u64,
    now: i64,
) -> u64 {
    if let Some(idx) = subagent_focus {
        match chat.blocks.get(idx) {
            Some(ChatBlock::Subagent {
                started_at_ms,
                done: false,
                ..
            }) => ((now - *started_at_ms).max(0)) as u64,
            _ => 0,
        }
    } else {
        run_elapsed_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subagent_block(started_at_ms: i64, done: bool) -> ChatBlock {
        ChatBlock::Subagent {
            id: "sub-1".to_string(),
            child_session_id: "child-1".to_string(),
            kind: "explore".to_string(),
            prompt: "test".to_string(),
            view: ChatView::default(),
            done,
            ok: false,
            cancelled: false,
            summary: String::new(),
            started_at_ms,
            elapsed_ms: None,
        }
    }

    #[test]
    fn running_subagent_shows_live_elapsed() {
        let mut chat = ChatView::default();
        chat.blocks.push(subagent_block(1000, false));
        // now=5000, started_at_ms=1000 -> 4000
        assert_eq!(display_turn_ms(&chat, Some(0), 999, 5000), 4000);
    }

    #[test]
    fn done_subagent_returns_zero() {
        let mut chat = ChatView::default();
        chat.blocks.push(subagent_block(1000, true));
        assert_eq!(display_turn_ms(&chat, Some(0), 999, 5000), 0);
    }

    #[test]
    fn no_subagent_focus_falls_back_to_run_elapsed() {
        let chat = ChatView::default();
        assert_eq!(display_turn_ms(&chat, None, 42000, 5000), 42000);
    }
}
