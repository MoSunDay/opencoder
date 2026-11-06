//! Live disclosure policy for the step ladder — WHEN the automatic
//! open/collapse of `StepGroup` levels happens during streaming. The WHAT
//! (which flag renders which row) stays in `chat_steps.rs` /
//! `chat_step_render.rs`; this module only moves the flags, mirroring the
//! SPA's `steps/useLadderDisclosure.js` auto-open keys:
//!
//! * `activate` — while the turn runs, the group row and the LAST (running)
//!   step auto-open, so reasoning deltas and call rows stream in place
//!   instead of hiding behind the collapsed `▸ N Steps` row. Called from
//!   `steps::set_turn_progress(.., true)` (every live `ToolStart`) and the
//!   fresh-step branch of `steps::append_step_thinking_delta`.
//! * `settle` — the running position retires (Say's first chunk, `Done`,
//!   `Error`, new-turn admission): every auto-managed level re-collapses to
//!   the `N Steps + Say` summary. Called from
//!   `steps::set_turn_progress(.., false)`.
//! * `settle_steps_before_last` — a new step was born: the finished steps
//!   above collapse while the fresh running step takes over the open slot.
//!
//! USER PRIORITY: a level the user clicked (or Ctrl+L) carries
//! `user_touched` and is never moved by this module again for the rest of
//! the turn. Replay/snapshot groups never pass through here at all — they
//! stay fully collapsed.

use super::{ChatBlock, Step};

/// Whether the open ladder is the AUTO-managed running position (opened by
/// `activate`, never touched by the user). While true the renderer shows
/// only the running (last) step's rows: earlier steps of the same turn are
/// already summarized by the `N Steps` count, and rendering one header row
/// per step floods the viewport on long tool loops. A user-opened ladder
/// (`!cmd`, a click on the turn row) carries `user_touched` and keeps the
/// full step list. `settle` collapses the group back to the `N Steps + Say`
/// summary either way.
pub(crate) fn running_step_only(open: bool, user_touched: bool) -> bool {
    open && !user_touched
}

/// First step row index the renderer draws — and therefore the only rows the
/// click targets may reference. `0` for a user-managed ladder (every step row
/// renders); `n - 1` for the auto-managed running position (only the last).
/// Renderer, header accounting and `visible_targets` all derive their skip
/// from this one function so the three line-count mirrors cannot drift.
pub(crate) fn first_visible_step(n: usize, open: bool, user_touched: bool) -> usize {
    if running_step_only(open, user_touched) {
        n.saturating_sub(1)
    } else {
        0
    }
}

/// Index of the canonical `StepGroup` of the admitted turn: the FIRST group
/// at/after the turn floor, ignoring presentation blocks — the same target
/// `steps::set_turn_progress` addresses, so animation and disclosure can
/// never disagree about which ladder is live.
fn canonical_group_idx(blocks: &[ChatBlock], turn_start: usize) -> Option<usize> {
    let floor = turn_start.min(blocks.len());
    blocks[floor..]
        .iter()
        .position(|block| matches!(block, ChatBlock::StepGroup { .. }))
        .map(|relative| floor + relative)
}

/// Auto-open the live turn's running position: group row + last step + that
/// step's calls aggregate. User-touched levels keep their exact state.
pub(crate) fn activate(blocks: &mut [ChatBlock], turn_start: usize) {
    let Some(idx) = canonical_group_idx(blocks, turn_start) else {
        return;
    };
    if let ChatBlock::StepGroup {
        open,
        user_touched,
        steps,
        ..
    } = &mut blocks[idx]
    {
        if !*user_touched {
            *open = true;
        }
        if let Some(step) = steps.last_mut() {
            open_step(step);
        }
    }
}

/// Retire the live turn's auto-disclosure: collapse the group row and every
/// auto-managed step back to the folded summary. User-touched levels keep
/// their state (e.g. a call output the user pinned open stays open behind
/// the collapsed group row).
pub(crate) fn settle(blocks: &mut [ChatBlock], turn_start: usize) {
    let Some(idx) = canonical_group_idx(blocks, turn_start) else {
        return;
    };
    if let ChatBlock::StepGroup {
        open,
        user_touched,
        steps,
        ..
    } = &mut blocks[idx]
    {
        if !*user_touched {
            *open = false;
        }
        for step in steps.iter_mut() {
            close_step(step);
        }
    }
}

/// A new step was just born: every AUTO-managed step above the (new) last
/// one is finished — collapse it so the running position stays singular.
pub(crate) fn settle_steps_before_last(steps: &mut [Step]) {
    if steps.len() < 2 {
        return;
    }
    let last = steps.len() - 1;
    for step in steps[..last].iter_mut() {
        close_step(step);
    }
}

fn open_step(step: &mut Step) {
    if step.user_touched {
        return;
    }
    step.open = true;
    step.calls_open = true;
    // Materialize the streamed markdown once on first exposure so the open
    // step immediately shows its accumulated thinking.
    super::steps::render_step_thinking(step);
}

fn close_step(step: &mut Step) {
    if step.user_touched {
        return;
    }
    step.open = false;
    step.calls_open = false;
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Line;

    fn step(open: bool, calls_open: bool, user_touched: bool) -> Step {
        Step {
            thinking_raw: String::new(),
            thinking: Vec::new(),
            thinking_dirty: false,
            calls: Vec::new(),
            open,
            calls_open,
            sealed: true,
            user_touched,
        }
    }

    fn group(steps: Vec<Step>, open: bool, user_touched: bool) -> ChatBlock {
        ChatBlock::StepGroup {
            steps,
            open,
            progress_active: false,
            user_touched,
        }
    }

    fn blocks(group_at: usize) -> Vec<ChatBlock> {
        let mut v = vec![ChatBlock::User { rendered: Vec::new() }];
        v.insert(group_at, group(vec![], false, false));
        v
    }

    fn disclosure(v: &[ChatBlock]) -> (bool, Vec<(bool, bool)>) {
        v.iter()
            .find_map(|b| match b {
                ChatBlock::StepGroup { open, steps, .. } => {
                    Some((*open, steps.iter().map(|s| (s.open, s.calls_open)).collect()))
                }
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn activate_opens_group_and_last_step() {
        let mut v = blocks(1);
        if let ChatBlock::StepGroup { steps, .. } = &mut v[1] {
            steps.push(step(false, false, false));
            steps.push(step(false, false, false));
        }
        activate(&mut v, 1);
        // Only the LAST (running) step opens; the finished one stays folded.
        assert_eq!(disclosure(&v), (true, vec![(false, false), (true, true)]));
    }

    #[test]
    fn activate_respects_user_touched_levels() {
        let mut v = blocks(1);
        if let ChatBlock::StepGroup { steps, .. } = &mut v[1] {
            steps.push(step(false, false, false));
        }
        // User collapsed the group row and the step earlier.
        if let ChatBlock::StepGroup {
            user_touched, steps, ..
        } = &mut v[1]
        {
            *user_touched = true;
            steps[0].user_touched = true;
        }
        activate(&mut v, 1);
        assert_eq!(disclosure(&v), (false, vec![(false, false)]));
    }

    #[test]
    fn settle_collapses_auto_levels_but_keeps_user_levels() {
        let mut v = blocks(1);
        if let ChatBlock::StepGroup { steps, .. } = &mut v[1] {
            steps.push(step(true, true, false));
            steps.push(step(true, true, true));
        }
        settle(&mut v, 1);
        assert_eq!(disclosure(&v), (false, vec![(false, false), (true, true)]));
    }

    #[test]
    fn settle_before_last_skips_user_steps_and_short_lists() {
        let mut steps = vec![
            step(true, true, false),
            step(true, true, true),
            step(false, false, false),
        ];
        settle_steps_before_last(&mut steps);
        assert_eq!(
            steps
                .iter()
                .map(|s| (s.open, s.calls_open))
                .collect::<Vec<_>>(),
            vec![(false, false), (true, true), (false, false)]
        );
        // Fewer than two steps: nothing to do, no panic.
        let mut one = vec![step(true, true, false)];
        settle_steps_before_last(&mut one);
        assert!(one[0].open && one[0].calls_open);
    }

    #[test]
    fn running_step_only_needs_an_untouched_open_ladder() {
        assert!(!running_step_only(false, false), "closed group");
        assert!(!running_step_only(false, true), "closed, user-touched");
        assert!(running_step_only(true, false), "auto-opened running position");
        assert!(!running_step_only(true, true), "user pinned the open ladder");
        // The derived skip: auto focus starts at the last step, a user
        // ladder starts at 0, and an empty turn cannot underflow.
        assert_eq!(first_visible_step(3, true, false), 2);
        assert_eq!(first_visible_step(3, true, true), 0);
        assert_eq!(first_visible_step(3, false, false), 0);
        assert_eq!(first_visible_step(0, true, false), 0);
    }

    #[test]
    fn canonical_group_targets_first_group_after_floor() {
        let v = vec![
            ChatBlock::User { rendered: Vec::new() },
            group(vec![], false, false),
            ChatBlock::Marker(vec![Line::from("m")]),
            group(vec![], false, false),
        ];
        assert_eq!(canonical_group_idx(&v, 0), Some(1));
        assert_eq!(canonical_group_idx(&v, 2), Some(3));
        // No group after the floor: nothing to manage.
        assert_eq!(canonical_group_idx(&v, 4), None);
    }
}
