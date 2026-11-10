//! The TURN LIVE REGION — rendered INLINE inside the transcript window:
//! while the admitted turn runs (`progress_active` on its group, i.e. no
//! Say has begun yet), the region streams the CURRENT (last, in-flight)
//! step ONLY — emitted by `flatten_with` (`chat_flatten.rs`) directly
//! BELOW the running group's ladder and ABOVE any later blocks (e.g.
//! subagents), inside the SAME scroll window as the rest of the
//! transcript. NO step counter row, NO Say: the moment a Say BEGINS the
//! group's `progress_active` disarms (see the stream layer) and the
//! region CLEARS; the open Say itself is TRANSCRIPT CONTENT while it
//! streams (raw body rows under the standalone `say:` header, the ladder
//! folded — the merge happens only on seal) and simply seals in place at
//! turn end, where the region has already cleared. The ladder above the
//! region renders SETTLED history only (`steps::settled_steps`): while
//! the FIRST step is still in flight the block itself emits ZERO rows and
//! the region is the turn's only surface.
//!
//! The region opens with a `> streaming` header row (`chat.rs`
//! `LIVE_ROW_OPEN_PREFIX`) — a click target (`LiveHeader`, toggles
//! `ChatView::live_collapsed`): collapsed it is the region's ONLY row
//! (`▸ streaming`); expanded (the default — every newly-admitted running turn
//! re-expands it) the current step's detail rows follow. Ctrl+L collapses
//! it with every other disclosure level.
//!
//! Call rendering here is IDENTICAL to the transcript ladder
//! (`chat_step_render.rs` — shared `push_step_details`, calls forced open):
//! the `N Function call(s)` aggregate row plus each call's header row (a
//! still-running call carries the spinner), the exact shape of the
//! ladder's open step. The ONE difference: results NEVER render here
//! (`show_results = false`) — `output` and `expanded` are both ignored;
//! the ladder owns every result drill-down.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

#[cfg(test)]
use super::ChatBlock;
use super::{step_render, theme, Step, GROUP_ROW_CLOSED_PREFIX, LIVE_ROW_OPEN_PREFIX};

/// The live region's data source: the group inside the CURRENT turn floor
/// whose `progress_active` is armed (i.e. no Say has begun yet — a Say
/// beginning disarms it, turn end settles it). Replay/snapshot and settled
/// turns never arm it, so it is a live-stream-only area.
#[cfg(test)]
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

/// The live region's full line buffer: the `> streaming` / `▸ streaming` header row
/// that marks the region's start (it is NOT transcript content), then —
/// while expanded (`collapsed == false`, the default) — ONLY the current
/// step's detail rows, rendered by the SAME shared renderer as the ladder
/// (`push_step_details`: thinking header + text, the calls aggregate row,
/// each call header; calls open, results NEVER here — the ladder owns the
/// drill-down). Collapsed the header row is the region's whole output.
pub(crate) fn render_lines(steps: &[Step], anim_tick: u32, collapsed: bool) -> Vec<Line<'static>> {
    let prefix = if collapsed {
        GROUP_ROW_CLOSED_PREFIX
    } else {
        LIVE_ROW_OPEN_PREFIX
    };
    let mut out = vec![Line::from(Span::styled(
        format!("{prefix}streaming"),
        Style::default()
            .fg(theme::warn_color())
            .add_modifier(Modifier::DIM),
    ))];
    if collapsed {
        return out;
    }
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
