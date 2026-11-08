//! The TURN LIVE REGION — the TAIL of the transcript's scroll window
//! (`render.rs`): rides the SAME scrollbar as the Say/transcript rows, one
//! window above the `[tok cost]` bottom border, deliberately OUTSIDE the
//! transcript content: while the admitted turn runs (`progress_active` on
//! its group, i.e. no Say has begun yet), this region streams the CURRENT
//! (last) step ONLY — its thinking header + text and the function-call
//! rows. NO step counter row, NO Say: the moment a Say BEGINS the group's
//! `progress_active` disarms (see the stream layer) and the region
//! CLEARS; the open Say itself is TRANSCRIPT CONTENT while it streams
//! (raw body rows under the standalone `say:` header, the ladder folded —
//! the merge happens only on seal) and simply seals in place at turn end,
//! where the region has already cleared.
//!
//! Call rendering here is IDENTICAL to the transcript ladder
//! (`chat_step_render.rs` — shared `push_step_details`, calls forced open):
//! the `N Function call(s)` aggregate row plus each call's header row (a
//! still-running call carries the spinner), the exact shape of the
//! ladder's open step. The ONE difference: results NEVER render here
//! (`show_results = false`) — `output` and `expanded` are both ignored;
//! the ladder owns every result drill-down.
//!
//! Zero coupling with ladder disclosure: `flatten_with` does NOT emit
//! these lines, so scroll, selection, hit-rects and line accounting
//! (`chat_headers.rs`) never see the strip.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::{step_render, theme, ChatBlock, Step};

/// The live region's data source: the group inside the CURRENT turn floor
/// whose `progress_active` is armed (i.e. no Say has begun yet — a Say
/// beginning disarms it, turn end settles it). Replay/snapshot and settled
/// turns never arm it, so it is a live-stream-only area.
pub(crate) fn live_data(blocks: &[ChatBlock], turn_start: usize) -> Option<&[Step]> {
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
/// step's detail rows, rendered by the SAME shared renderer as the ladder
/// (`push_step_details`: thinking header + text, the calls aggregate row,
/// each call header; calls open, results NEVER here — the ladder owns the
/// drill-down). Render-only: NO click targets.
pub(crate) fn render_lines(steps: &[Step], anim_tick: u32) -> Vec<Line<'static>> {
    let mut out = vec![Line::from(Span::styled(
        "\u{2b25} live",
        Style::default()
            .fg(theme::warn_color())
            .add_modifier(Modifier::DIM),
    ))];
    if let Some(si) = steps.len().checked_sub(1) {
        // Ladder-identical call rendering: the aggregate row + header rows
        // come straight from the shared step renderer (calls forced open);
        // `show_results = false` keeps every result out of this region.
        step_render::push_step_details(
            &mut out,
            si,
            &steps[si],
            steps.len(),
            anim_tick,
            true,
            false,
        );
    }
    out
}
