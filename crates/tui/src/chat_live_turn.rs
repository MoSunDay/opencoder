//! The TURN LIVE AREA — an independent region at the tail of the
//! transcript, deliberately OUTSIDE every ladder: while the admitted turn
//! runs (`progress_active` on its group, i.e. no Say has begun yet), this
//! area streams the CURRENT (last) step (step row + thinking + calls,
//! running calls carry the spinner). When the turn settles (Say first
//! chunk / Done / Error / new turn) the area disappears and the step lives
//! only inside that turn's FOLDED ladder; if the next turn immediately
//! starts running, the area simply shows that turn's current step.
//!
//! Zero coupling with ladder disclosure: the ladder blocks render pure
//! user-controlled fold state (`chat_step_render.rs`); opening or folding
//! them never changes this area, and this area never renders "inside" a
//! ladder block. It adds NO click targets and no line accounting is
//! needed: it is the transcript tail, so nothing follows it and
//! `collect_headers` (`chat_headers.rs`) is unaffected.

use ratatui::text::Line;

use super::{step_render, ChatBlock, Step};

/// The running turn's steps, if any: the group inside the CURRENT turn
/// floor whose `progress_active` is armed. Replay/snapshot and settled
/// turns never arm it, so the live area is a live-stream-only region.
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

/// Render the live area: the current step's body forced open (identical
/// shapes to a user-expanded ladder step, via the shared step-body
/// renderer) plus one trailing blank. Appended AFTER every transcript
/// block by `flatten_with` — a separate area, never inside a ladder.
pub(crate) fn flatten(out: &mut Vec<Line<'static>>, steps: &[Step], anim_tick: u32) {
    let si = steps.len() - 1;
    step_render::push_step_body(out, si, &steps[si], steps.len(), anim_tick, true, true);
    out.push(Line::from(""));
}
