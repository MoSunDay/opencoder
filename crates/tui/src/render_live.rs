//! The TURN LIVE REGION's render-side helpers. The region is the bottom
//! strip INSIDE the transcript frame — hugging the transcript's last
//! rendered row, above the `[tok cost]` bottom border
//! (`render.rs::render_body`) — and render-only: never part of
//! `flatten_with`, so scroll, selection, hit-rects and line accounting
//! never see it (`chat_live_turn.rs`).

use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::chat::ChatView;

/// The running turn's live lines. Empty when no turn runs or the plan
/// editor is active. UNTRUNCATED by policy: the caller hugs them right
/// below the transcript's content and only physical frame overflow clips
/// (see [`strip_rect`] / [`render_strip`]).
pub(crate) fn region(chat: &ChatView, plan_active: bool, anim_tick: u32) -> Vec<Line<'static>> {
    if plan_active {
        return Vec::new();
    }
    crate::chat::live_turn::live_data(&chat.blocks, chat.turn_block_start)
        .map(|live| crate::chat::live_turn::render_lines(live.steps, live.say, anim_tick))
        .unwrap_or_default()
}

/// The strip's rectangle: HUG the transcript's last rendered row — start
/// right below the content (`used = min(content_rows, inner.height)`, so a
/// scrolled/full transcript keeps the strip at the frame's tail) with NO
/// height cap and NO reserved slice; the strip is clipped only by the
/// frame's physical bottom. `None` when there is nothing to render.
pub(crate) fn strip_rect(inner: Rect, content_rows: usize, live_len: usize) -> Option<Rect> {
    let used = content_rows.min(inner.height as usize);
    let y = inner.y + used as u16;
    let avail = inner.bottom().saturating_sub(y) as usize;
    let h = live_len.min(avail);
    (h > 0).then_some(Rect {
        x: inner.x,
        y,
        width: inner.width,
        height: h as u16,
    })
}

/// Render the strip, tail-kept: the newest lines matter while streaming
/// (the tail-slice only engages at physical frame overflow).
pub(crate) fn render_strip(f: &mut Frame, lines: &[Line<'static>], area: Rect) {
    let skip = lines.len().saturating_sub(area.height as usize);
    f.render_widget(
        Paragraph::new(lines[skip..].to_vec()).wrap(Wrap { trim: false }),
        area,
    );
}
