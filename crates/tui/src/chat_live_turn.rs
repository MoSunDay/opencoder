//! The TURN LIVE REGION — the bottom strip INSIDE the transcript frame
//! (`render.rs`), directly below the Say rows and ABOVE the `[tok cost]`
//! bottom border, deliberately OUTSIDE the transcript content:
//! while the admitted turn runs (`progress_active` on its group, i.e. no
//! Say has begun yet), this region streams the CURRENT (last) step (step
//! row + thinking + calls, running calls carry the spinner). When the turn
//! settles (Say first chunk / Done / Error / new turn) the region clears
//! and the step lives only inside that turn's FOLDED ladder; if the next
//! turn immediately starts running, the region simply shows that turn's
//! current step.
//!
//! Zero coupling with ladder disclosure AND with the transcript itself:
//! `flatten_with` does NOT emit these lines (the transcript only ever
//! holds Say + ladder blocks), so scroll, selection, hit-rects and line
//! accounting (`chat_headers.rs`) are all untouched by streaming. The
//! ladder blocks render pure user-controlled fold state
//! (`chat_step_render.rs`); opening or folding them never changes this
//! region, and this region never renders "inside" a ladder block.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::{step_render, theme, ChatBlock, Step};

/// The running turn's steps, if any: the group inside the CURRENT turn
/// floor whose `progress_active` is armed. Replay/snapshot and settled
/// turns never arm it, so the live region is a live-stream-only area.
pub(crate) fn active_steps(blocks: &[ChatBlock], turn_start: usize) -> Option<&[Step]> {
    let floor = turn_start.min(blocks.len());
    blocks[floor..]
        .iter()
        .find_map(|block| match block {
            ChatBlock::StepGroup {
                steps,
                progress_active: true,
                ..
            } => Some(steps.as_slice()),
            _ => None,
        })
        .filter(|steps| !steps.is_empty())
}

/// The live region's full line buffer: a dim header rule that marks the
/// region's start (it is NOT transcript content), then ONLY the current
/// step's detail rows — thinking + call rows, NO step counter row (the
/// transcript ladder owns the step counting). Render-only: NO click targets.
pub(crate) fn render_lines(steps: &[Step], anim_tick: u32) -> Vec<Line<'static>> {
    let mut out = vec![Line::from(Span::styled(
        "\u{2b25} live",
        Style::default()
            .fg(theme::warn_color())
            .add_modifier(Modifier::DIM),
    ))];
    let si = steps.len() - 1;
    step_render::push_step_details(&mut out, si, &steps[si], steps.len(), anim_tick, true);
    out
}
