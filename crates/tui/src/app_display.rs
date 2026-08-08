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

/// Tail-duration timer value shown at the tail of the body content.
///
/// - Running subagent focused → its live elapsed (`now - started_at_ms`).
/// - No subagent focused → elapsed of the current run of consecutive tool
///   calls (a "round"), measured from the first call's start (see
///   [`call_round_ms`]). Disappears as soon as the last tool finishes.
/// - Done/finished subagent focused, or invalid index → 0.
pub(super) fn display_tail_ms(chat: &ChatView, subagent_focus: Option<usize>, now: i64) -> u64 {
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
        call_round_ms(chat, now)
    }
}

/// Milliseconds since the first call of the current round of tool calls
/// started; 0 when no tool is currently running.
///
/// A round is the contiguous tail run of `Tool` blocks ending in a *running*
/// one (`elapsed_ms == None`). Its start is the first block of that segment,
/// so a burst of consecutive calls is timed as one unit from its beginning.
/// Any non-Tool block (text, marker, …) or a finished round truncates it.
fn call_round_ms(chat: &ChatView, now: i64) -> u64 {
    let Some(last_running) = chat.blocks.iter().rposition(|b| {
        matches!(b, ChatBlock::Tool { elapsed_ms: None, .. })
    }) else {
        return 0;
    };
    let mut start = last_running;
    while start > 0 && matches!(chat.blocks[start - 1], ChatBlock::Tool { .. }) {
        start -= 1;
    }
    let started_at_ms = match &chat.blocks[start] {
        ChatBlock::Tool { started_at_ms, .. } => *started_at_ms,
        _ => unreachable!("blocks[start] is a Tool by construction"),
    };
    ((now - started_at_ms).max(0)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Line;

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

    fn tool_block(started_at_ms: i64, elapsed_ms: Option<u64>) -> ChatBlock {
        ChatBlock::Tool {
            id: "t1".to_string(),
            header: Line::from("bash ls"),
            output: Vec::new(),
            collapsed: true,
            started_at_ms,
            elapsed_ms,
        }
    }

    #[test]
    fn running_subagent_shows_live_elapsed() {
        let mut chat = ChatView::default();
        chat.blocks.push(subagent_block(1000, false));
        // now=5000, started_at_ms=1000 -> 4000
        assert_eq!(display_tail_ms(&chat, Some(0), 5000), 4000);
    }

    #[test]
    fn done_subagent_returns_zero() {
        let mut chat = ChatView::default();
        chat.blocks.push(subagent_block(1000, true));
        assert_eq!(display_tail_ms(&chat, Some(0), 5000), 0);
    }

    #[test]
    fn no_running_tool_returns_zero() {
        // Empty chat: nothing running.
        let chat = ChatView::default();
        assert_eq!(display_tail_ms(&chat, None, 5000), 0);
        // Only finished tools: the round is over, tail timer must vanish.
        let mut chat = ChatView::default();
        chat.blocks.push(tool_block(1000, Some(3000)));
        assert_eq!(display_tail_ms(&chat, None, 5000), 0);
    }

    #[test]
    fn running_tool_round_shows_elapsed() {
        let mut chat = ChatView::default();
        chat.blocks.push(tool_block(1000, None));
        // now=5000, started_at_ms=1000 -> 4000
        assert_eq!(display_tail_ms(&chat, None, 5000), 4000);
    }

    #[test]
    fn round_spans_consecutive_tools_from_first_start() {
        let mut chat = ChatView::default();
        // A burst: first call finished, second finished, third still running.
        chat.blocks.push(tool_block(1000, Some(2000)));
        chat.blocks.push(tool_block(2000, Some(3000)));
        chat.blocks.push(tool_block(3000, None));
        // The round is timed from the FIRST call of the burst (1000) to now.
        assert_eq!(display_tail_ms(&chat, None, 6000), 5000);
    }

    #[test]
    fn non_tool_block_breaks_the_round() {
        let mut chat = ChatView::default();
        chat.blocks.push(tool_block(1000, None));
        chat.blocks.push(ChatBlock::Marker(Vec::new()));
        chat.blocks.push(tool_block(4000, None));
        // The earlier tool is separated by a non-Tool block: the round only
        // spans the tail segment, starting at the last tool (4000).
        assert_eq!(display_tail_ms(&chat, None, 6000), 2000);
    }
}
