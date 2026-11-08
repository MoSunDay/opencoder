//! The TURN LIVE REGION — the bottom strip INSIDE the transcript frame
//! (`render.rs`), directly below the Say rows and ABOVE the `[tok cost]`
//! bottom border, deliberately OUTSIDE the transcript content:
//! while the admitted turn runs (`progress_active` on its group, i.e. no
//! Say has begun yet), this region streams the CURRENT (last) step
//! (thinking + calls). When the turn settles (Say first chunk / Done /
//! Error / new turn) the region clears and the step lives only inside that
//! turn's FOLDED ladder; if the next turn immediately starts running, the
//! region simply shows that turn's current step.
//!
//! Call rendering here is DELIBERATELY asymmetric with the transcript
//! ladder (`chat_step_render.rs`): every call renders a name row with the
//! call's FULL input text EXPANDED below it (the live region is a reading
//! surface, not a drill-down), while call RESULTS stay COLLAPSED —
//! `output` is NEVER rendered here and `expanded` is ignored entirely; the
//! ladder owns every result drill-down.
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

use super::{step_render, theme, ChatBlock, Step, GROUP_ROW_OPEN_PREFIX, SPINNER};

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
/// step's detail rows — the thinking block plus per-call name rows with
/// each call's FULL input expanded below (NO step counter row, NO calls
/// aggregate row, NO call results: the transcript ladder owns all three).
/// Render-only: NO click targets.
pub(crate) fn render_lines(steps: &[Step], anim_tick: u32) -> Vec<Line<'static>> {
    let mut out = vec![Line::from(Span::styled(
        "\u{2b25} live",
        Style::default()
            .fg(theme::warn_color())
            .add_modifier(Modifier::DIM),
    ))];
    let step = &steps[steps.len() - 1];
    step_render::push_thinking(&mut out, step);
    for c in &step.calls {
        // Name row: 4-space indent + the open glyph + the tool name, the
        // same accent-bold style as the ladder's call header rows. A still
        // running call (`ToolEnd` not yet seen) carries the motion spinner.
        let mut spans = vec![
            Span::raw("    "),
            Span::styled(
                format!("{GROUP_ROW_OPEN_PREFIX}{}", c.name),
                Style::default()
                    .fg(theme::accent())
                    .add_modifier(Modifier::BOLD),
            ),
        ];
        if c.elapsed_ms.is_none() {
            spans.push(Span::styled(
                format!(" {}", SPINNER[(anim_tick as usize) % SPINNER.len()]),
                Style::default().fg(theme::warn_color()),
            ));
        }
        out.push(Line::from(spans));
        // The call's FULL input, EXPANDED below the name row. Results stay
        // COLLAPSED here — `c.output` is never rendered and `c.expanded`
        // is ignored: the ladder owns the result drill-down.
        if !c.input.is_empty() {
            let input_lines = c
                .input
                .lines()
                .map(|l| {
                    Line::from(Span::styled(
                        l.to_string(),
                        Style::default().fg(theme::muted()),
                    ))
                })
                .collect::<Vec<_>>();
            out.extend(super::types::indented(&input_lines, 8));
        }
    }
    out
}
