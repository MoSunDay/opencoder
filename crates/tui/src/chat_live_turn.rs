//! The TURN LIVE REGION — the bottom strip INSIDE the transcript frame
//! (`render.rs`), hugging the transcript's last rendered row and ABOVE the
//! `[tok cost]` bottom border, deliberately OUTSIDE the transcript content:
//! while the admitted turn runs (`progress_active` on its group, i.e. no
//! Say has begun yet), this region streams the CURRENT (last) step
//! (thinking + calls). Once a Say BEGINS it is no longer transcript
//! content either: the open (`done: false`) Say streams HERE (a `💬 Say`
//! marker + its RAW text, alongside the turn's steps) and only lands in
//! the transcript once sealed (`done: true`, via the existing finalize
//! points — Done / Error / a new Say / a following block). When the turn
//! settles the region clears and everything lives inside that turn's
//! FOLDED ladder; if the next turn immediately starts running, the region
//! simply shows that turn's current step.
//!
//! Call rendering here is IDENTICAL to the transcript ladder
//! (`chat_step_render.rs` — shared `push_step_details`, calls forced open):
//! the `N Function call(s)` aggregate row plus each call's header row (a
//! still-running call carries the spinner), the exact shape of the
//! ladder's open step. The ONE difference: results NEVER render here
//! (`show_results = false`) — `output` and `expanded` are both ignored;
//! the ladder owns every result drill-down.
//!
//! Zero coupling with ladder disclosure AND with the transcript itself:
//! `flatten_with` does NOT emit these lines (the transcript only ever
//! holds sealed Say + ladder blocks), so scroll, selection, hit-rects and
//! line accounting (`chat_headers.rs`) are all untouched by streaming. The
//! ladder blocks render pure user-controlled fold state
//! (`chat_step_render.rs`); opening or folding them never changes this
//! region, and this region never renders "inside" a ladder block.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::{step_render, theme, ChatBlock, Step};

/// The live region's data snapshot: the running turn's steps (if any) plus
/// the open Say's RAW text (if any). Both are optional and independent —
/// a turn may stream only steps (pre-Say), only a Say (call-less turn), or
/// both at once.
pub(crate) struct LiveTurn<'a> {
    pub steps: Option<&'a [Step]>,
    pub say: Option<&'a str>,
}

/// The phase-A source: the group inside the CURRENT turn floor whose
/// `progress_active` is armed (i.e. no Say has begun yet). Replay/snapshot
/// and settled turns never arm it, so it is a live-stream-only area.
fn active_steps(blocks: &[ChatBlock], turn_start: usize) -> Option<&[Step]> {
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

/// Gather the live region's data. Phase B — an open Say exists SOMEWHERE:
/// walk the blocks BACKWARDS from the end; the first `Assistant` with
/// `done: false` the walk crosses is the streaming Say (`say`), and the
/// walk CONTINUES (crossing `Assistant` blocks of any done-ness) until the
/// nearest `StepGroup` with non-empty steps (`steps`); any other block
/// kind (User/Marker/Subagent/Compaction/…) stops it with `steps = None`.
/// Phase A fallback — no open Assistant anywhere: today's pre-Say logic
/// (the `progress_active` group within `blocks[turn_start..]`) arms
/// `steps` alone. `None` when neither phase yields anything.
pub(crate) fn live_data<'a>(blocks: &'a [ChatBlock], turn_start: usize) -> Option<LiveTurn<'a>> {
    if !blocks
        .iter()
        .any(|block| matches!(block, ChatBlock::Assistant { done: false, .. }))
    {
        return active_steps(blocks, turn_start).map(|steps| LiveTurn {
            steps: Some(steps),
            say: None,
        });
    }
    let mut say: Option<&str> = None;
    let mut steps: Option<&[Step]> = None;
    for block in blocks.iter().rev() {
        match block {
            ChatBlock::Assistant {
                raw, done: false, ..
            } => {
                if say.is_none() {
                    say = Some(raw.as_str());
                }
            }
            ChatBlock::Assistant { .. } => {}
            ChatBlock::StepGroup {
                steps: group_steps, ..
            } => {
                if !group_steps.is_empty() {
                    steps = Some(group_steps.as_slice());
                }
                break;
            }
            _ => break,
        }
    }
    (say.is_some() || steps.is_some()).then_some(LiveTurn { steps, say })
}

/// The live region's full line buffer: a dim header rule that marks the
/// region's start (it is NOT transcript content), then — when the turn has
/// steps — ONLY the current step's detail rows, rendered by the SAME
/// `push_step_details` as the ladder's open step (thinking block, the
/// `N Function call(s)` aggregate row, each call's header row — NO step
/// counter row, and `show_results = false`: call results NEVER render
/// here, the ladder owns the drill-down) — and then, while a Say streams
/// (`done: false`), its `💬 Say` marker row plus the RAW text lines below
/// (the Say lands in the transcript only once sealed). Render-only: NO
/// click targets.
pub(crate) fn render_lines(
    steps: Option<&[Step]>,
    say: Option<&str>,
    anim_tick: u32,
) -> Vec<Line<'static>> {
    let mut out = vec![Line::from(Span::styled(
        "\u{2b25} live",
        Style::default()
            .fg(theme::warn_color())
            .add_modifier(Modifier::DIM),
    ))];
    if let Some(steps) = steps.filter(|steps| !steps.is_empty()) {
        let si = steps.len() - 1;
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
    // The STREAMING Say: a marker row styled like the step's thinking
    // header (`STEP_THINKING_HEADER`: pink + bold), then the RAW text
    // lines indented 8 — low-latency reading, no markdown while open (the
    // sealed body lands in the transcript later).
    if let Some(say) = say.filter(|say| !say.is_empty()) {
        out.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(
                "\u{1f4ac} Say",
                Style::default()
                    .fg(theme::pink())
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        let rows = say
            .lines()
            .map(|l| {
                Line::from(Span::styled(
                    l.to_string(),
                    Style::default().fg(theme::muted()),
                ))
            })
            .collect::<Vec<_>>();
        out.extend(super::types::indented(&rows, 8));
    }
    out
}
