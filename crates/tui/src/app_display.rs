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
