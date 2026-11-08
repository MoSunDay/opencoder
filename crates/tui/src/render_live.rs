//! The TURN LIVE REGION's render-side helpers. The region is the TAIL of
//! the transcript's scroll window (`render.rs::render_body`): its wrapped
//! rows ride the SAME scrollbar as the Say/transcript content — one
//! window, never truncated (scrolling up reveals the region's start), and
//! only physical frame overflow clips the tail. Still render-only: never
//! part of `flatten_with`, so selection, hit-rects and line accounting
//! never see it (`chat_live_turn.rs`).

use ratatui::text::Line;

use crate::chat::ChatView;
use crate::render_viewport::wrapped_rows;

/// The running turn's live lines. Empty when no turn runs or the plan
/// editor is active. UNTRUNCATED by policy: the caller rides them on the
/// transcript scroll window's tail (`render.rs::render_body`), so the
/// only clipping is physical frame overflow of that shared window.
pub(crate) fn region(chat: &ChatView, plan_active: bool, anim_tick: u32) -> Vec<Line<'static>> {
    if plan_active {
        return Vec::new();
    }
    crate::chat::live_turn::live_data(&chat.blocks, chat.turn_block_start)
        .map(|steps| crate::chat::live_turn::render_lines(steps, anim_tick))
        .unwrap_or_default()
}

/// The region's total WRAPPED row count at width `w` — the rows it adds
/// to the transcript scroll window (`wrapped_rows` counts what the
/// wrapping `Paragraph` actually renders, raw line counts under-count).
pub(crate) fn wrapped_total(lines: &[Line<'static>], w: u16) -> usize {
    lines.iter().map(|line| wrapped_rows(line, w)).sum()
}

/// Slice the region's window at width `w`: skip `skip_rows` whole wrapped
/// rows (the remainder inside the first surviving line becomes the
/// paragraph's `top_skip`), then take lines while the accumulated wrapped
/// rows stay under `budget_rows` — the last line may overflow the budget,
/// the rendering `Paragraph` clips it. Returns the surviving lines and
/// the row offset to scroll past inside the first of them.
pub(crate) fn slice_window(
    lines: &[Line<'static>],
    w: u16,
    skip_rows: usize,
    budget_rows: usize,
) -> (Vec<Line<'static>>, u16) {
    let mut idx = 0usize;
    let mut skip = skip_rows;
    while idx < lines.len() {
        let rows = wrapped_rows(&lines[idx], w);
        if rows > skip {
            break;
        }
        skip -= rows;
        idx += 1;
    }
    if idx >= lines.len() || budget_rows == 0 {
        return (Vec::new(), 0);
    }
    let top_skip = skip;
    let mut out = vec![lines[idx].clone()];
    let mut acc = wrapped_rows(&lines[idx], w);
    idx += 1;
    while acc < budget_rows && idx < lines.len() {
        acc += wrapped_rows(&lines[idx], w);
        out.push(lines[idx].clone());
        idx += 1;
    }
    (out, top_skip as u16)
}
