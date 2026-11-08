//! The TURN LIVE REGION's render-side helpers. The region is the bottom
//! strip INSIDE the transcript frame — directly below the Say rows and
//! ABOVE the `[tok cost]` bottom border (`render.rs::render_body`) — and
//! render-only: never part of `flatten_with`, so scroll, selection,
//! hit-rects and line accounting never see it (`chat_live_turn.rs`).

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::chat::ChatView;

/// The running turn's live lines plus their capped height (the same
/// one-third budget the composer uses). Empty when no turn runs or the
/// plan editor is active.
pub(crate) fn region(
    chat: &ChatView,
    plan_active: bool,
    anim_tick: u32,
    frame_h: u16,
) -> (Vec<Line<'static>>, u16) {
    let lines = if plan_active {
        Vec::new()
    } else {
        crate::chat::live_turn::active_steps(&chat.blocks, chat.turn_block_start)
            .map(|steps| crate::chat::live_turn::render_lines(steps, anim_tick))
            .unwrap_or_default()
    };
    let h = (lines.len() as u16).min(frame_h / 3);
    (lines, h)
}

/// Split the frame's inner area into (transcript part, live strip). The
/// strip is `None` when off (no running turn) or too small to fit.
pub(crate) fn split_inner(inner: Rect, live_h: u16) -> (Rect, Option<Rect>) {
    if live_h > 0 && inner.height > live_h {
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(live_h)])
            .split(inner);
        (parts[0], Some(parts[1]))
    } else {
        (inner, None)
    }
}

/// Render the strip, tail-kept: the newest lines matter while streaming.
pub(crate) fn render_strip(f: &mut Frame, lines: &[Line<'static>], area: Rect) {
    let skip = lines.len().saturating_sub(area.height as usize);
    f.render_widget(
        Paragraph::new(lines[skip..].to_vec()).wrap(Wrap { trim: false }),
        area,
    );
}
