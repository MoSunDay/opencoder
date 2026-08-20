use super::*;
use crate::chat::ChatView;
use opencoder_session::SessionEvent;
use ratatui::backend::TestBackend;

pub(super) fn thinking_view() -> ChatView {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("think-a-1\nthink-a-2".into()));
    v.apply(&SessionEvent::TextDelta("answer".into()));
    v.apply(&SessionEvent::Done);
    v
}

/// Collect the rendered text of a single buffer row into a String by
/// concatenating every cell's symbol.
pub(super) fn row_text(buf: &ratatui::buffer::Buffer, y: u16, width: u16) -> String {
    let mut s = String::new();
    for x in 0..width {
        if let Some(cell) = buf.cell((x, y)) {
            s.push_str(cell.symbol());
        }
    }
    s
}

mod arrow_click;
mod body;
mod chips;
mod compaction;
mod composer;
mod cursor;
mod cursor_popup;
mod queue_panel;
mod status_bar;
mod status_ctx;
mod thinking;
mod timer;
mod tok_cost;
