//! Replay canonicalization for `ChatBlock::StepGroup` — the replay half of
//! the shared step semantics in `chat_steps.rs` (one implementation used by
//! BOTH the live streaming path (`ChatView::apply`) and replay
//! (`session_ui::replay`), so step boundaries never drift between them).
//!
//! `coalesce_steps` re-runs the live path's thinking absorption (same
//! boundaries as `chat_steps::absorb_pending_thinking`) over replayed
//! blocks, then `normalize_turn_groups` collapses the result to one
//! StepGroup per Say-closed sub-turn — the same floor the live path
//! enforces by advancing `turn_block_start` below every newly-opened Say.

use ratatui::text::Line;

use super::steps::{absorb_pending_thinking, place_thinking_step, span_text};
use super::{ChatBlock, Step};

/// Fold replayed blocks: absorb each run of trailing `Thinking` blocks into
/// the following `StepGroup`'s first step, and merge runs of adjacent
/// `StepGroup`s into one group. The trailing run may sit further back,
/// behind the turn's own `Assistant` speech — those blocks are absorbed too
/// (same boundaries as `absorb_pending_thinking`), so a replayed tool turn
/// renders with the same step-local thinking as the live path. Thinking no
/// tool round ever consumes (pure-text rounds, a turn's trailing Say round)
/// folds into a call-less step at the same boundaries the live path flushes
/// at — user/subagent/marker pushes and the end of the list — so old
/// transcripts replay into the ladder identically. Pure w.r.t. everything
/// but the argument.
pub(crate) fn coalesce_steps(blocks: &mut Vec<ChatBlock>) {
    let mut out: Vec<ChatBlock> = Vec::with_capacity(blocks.len());
    let mut pending: Vec<ChatBlock> = Vec::new();
    for block in blocks.drain(..) {
        match block {
            ChatBlock::Thinking { .. } => pending.push(block),
            ChatBlock::StepGroup {
                mut steps,
                open,
                progress_active,
                user_touched,
            } => {
                if steps.is_empty() {
                    // Defensive: a zero-step group carries nothing; the
                    // pending thinking still joins the ladder.
                    place_thinking_step(&mut out, take_rendered_thinking(&mut pending));
                    continue;
                }
                // Thinking that trails inside `out` behind the turn's own
                // assistant speech belongs to this round as well.
                let mut absorbed = absorb_pending_thinking(&mut out);
                absorbed.append(&mut take_rendered_thinking(&mut pending));
                let mut absorbed_raw = span_text(&absorbed);
                absorbed_raw.push_str(&steps[0].thinking_raw);
                steps[0].thinking_raw = absorbed_raw;
                absorbed.append(&mut steps[0].thinking);
                steps[0].thinking = absorbed;
                steps[0].thinking_dirty = false;
                match out.last_mut() {
                    Some(ChatBlock::StepGroup {
                        steps: prev,
                        progress_active: prev_progress,
                        user_touched: prev_touched,
                        ..
                    }) => {
                        prev.append(&mut steps);
                        *prev_progress |= progress_active;
                        *prev_touched |= user_touched;
                    }
                    _ => out.push(ChatBlock::StepGroup {
                        steps,
                        open,
                        progress_active,
                        user_touched,
                    }),
                }
            }
            ChatBlock::Assistant { .. } => {
                // Speech is transparent for the pending thinking run — it
                // rides along until the next group (folds there) or the next
                // boundary / end of list (folds into a call-less step),
                // exactly like the live walk.
                out.push(block);
            }
            other => {
                place_thinking_step(&mut out, take_rendered_thinking(&mut pending));
                out.push(other);
            }
        }
    }
    place_thinking_step(&mut out, take_rendered_thinking(&mut pending));
    normalize_turn_groups(&mut out);
    *blocks = out;
}

/// Canonicalize replay into one StepGroup per Say-closed sub-turn.
/// Persisted sessions are message-pair streams, so tool-result carrier rows
/// can sit between tool rounds; those are presentation details, not Turn
/// boundaries. The canonical unit is the sub-turn a Say CLOSES — the same
/// floor the live path enforces by advancing `turn_block_start` below every
/// newly-opened Say (see `chat_stream.rs` and
/// `features/changelog/2026-09-03/say-closes-turn-transcript-reset-echo.md`)
/// — so a later ladder never merges back above the Say that closed its
/// turn, and the user echo still opens a fresh ladder. Each sub-segment's
/// groups merge into ONE ladder placed at the segment's first StepGroup
/// (a sub-segment holds no Assistant, so that position degrades from the
/// legacy first-Assistant rule), keeping the default `N Steps + Say` order.
fn normalize_turn_groups(blocks: &mut Vec<ChatBlock>) {
    fn append_segment(out: &mut Vec<ChatBlock>, segment: &mut Vec<ChatBlock>) {
        let insert_at = segment
            .iter()
            .position(|block| {
                matches!(
                    block,
                    ChatBlock::Assistant { .. } | ChatBlock::StepGroup { .. }
                )
            })
            .unwrap_or(segment.len());
        let mut steps: Vec<Step> = Vec::new();
        let mut open = false;
        let mut progress_active = false;
        let mut user_touched = false;
        for block in segment.iter_mut() {
            if let ChatBlock::StepGroup {
                steps: group_steps,
                open: group_open,
                progress_active: group_progress,
                user_touched: group_touched,
            } = block
            {
                for mut step in group_steps.drain(..) {
                    let has_thinking = !step.thinking_raw.is_empty() || !step.thinking.is_empty();
                    if !has_thinking {
                        if let Some(previous) = steps.last_mut() {
                            previous.calls.append(&mut step.calls);
                            previous.open |= step.open;
                            previous.calls_open |= step.calls_open;
                            previous.sealed &= step.sealed;
                            previous.user_touched |= step.user_touched;
                            continue;
                        }
                    }
                    steps.push(step);
                }
                open |= *group_open;
                progress_active |= *group_progress;
                user_touched |= *group_touched;
            }
        }
        if steps.is_empty() {
            out.append(segment);
            return;
        }
        let mut inserted = false;
        for (idx, block) in segment.drain(..).enumerate() {
            if idx == insert_at {
                out.push(ChatBlock::StepGroup {
                    steps: std::mem::take(&mut steps),
                    open,
                    progress_active,
                    user_touched,
                });
                inserted = true;
            }
            if !matches!(block, ChatBlock::StepGroup { .. }) {
                out.push(block);
            }
        }
        if !inserted {
            out.push(ChatBlock::StepGroup {
                steps,
                open,
                progress_active,
                user_touched,
            });
        }
    }

    let mut out = Vec::with_capacity(blocks.len());
    let mut segment = Vec::new();
    for block in blocks.drain(..) {
        if matches!(block, ChatBlock::User { .. } | ChatBlock::Assistant { .. }) {
            // The user echo opens a fresh ladder, and a Say CLOSES its
            // sub-turn: both cap the accumulated segment here, so
            // StepGroups that follow a Say can never merge back above it —
            // the replay mirror of the live path advancing
            // `turn_block_start` below every newly-opened Say.
            append_segment(&mut out, &mut segment);
            out.push(block);
        } else {
            segment.push(block);
        }
    }
    append_segment(&mut out, &mut segment);
    *blocks = out;
}

/// Pop the pending `Thinking` run, rendering each block's markdown.
fn take_rendered_thinking(pending: &mut Vec<ChatBlock>) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for block in pending.drain(..) {
        if let ChatBlock::Thinking { text, .. } = block {
            let mut rendered = crate::markdown::render(&text);
            lines.append(&mut rendered);
        }
    }
    lines
}
