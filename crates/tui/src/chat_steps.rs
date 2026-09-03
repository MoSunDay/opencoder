//! Shared step semantics for `ChatBlock::StepGroup` — one implementation used
//! by BOTH the live streaming path (`ChatView::apply`) and replay
//! (`session_ui::replay`), so step boundaries never drift between them.
//!
//! A step is one reasoning run plus every function call that follows it.
//! Function-call completion and provider-round boundaries do not split a
//! step: calls keep accumulating until the next reasoning run begins.
//!
//! A user turn owns exactly one `StepGroup`: live updates use the explicit
//! `turn_block_start` boundary, while replay canonicalizes message pairs at
//! real `User` blocks. Assistant Say and presentation blocks never split it.
//!
//! Thinking absorption: every round's reasoning lives strictly step-local —
//! the pending `Thinking` blocks trailing the flow (even behind the turn's
//! own `Assistant` speech) are folded into the step the next `ToolStart`
//! opens, and a round whose NO tool call ever follows (pure-text round, or a
//! turn's final Say round) is flushed into a call-less step at the run-end /
//! boundary pushes via `flush_pending_thinking`. Thinking therefore never
//! survives as a top-level block at rest. `coalesce_steps` applies the same
//! fold on replay.

use ratatui::text::Line;

use super::{ChatBlock, Step, ToolCall};

/// One clickable row inside a `StepGroup`'s three-level ladder: the turn row
/// (toggles all steps), a step row (toggles that step), a calls aggregation
/// row (toggles that step's call list), or a function-call row (toggles that
/// call's result).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StepTarget {
    Group,
    Step(usize),
    Calls(usize),
    Call(usize, usize),
}

/// The group's currently rendered click targets, in visual order: the group
/// row always; while the turn is open each step row; while a step is open,
/// its calls aggregation row; while that list is open, each function-call
/// row. Mirrors the `collect_headers` walk exactly, so
/// `ToolCallHeader::call_idx` indexes this list and `toggle_tool_call_at`
/// resolves the same row the renderer drew.
pub(crate) fn visible_targets(open: bool, steps: &[Step]) -> Vec<StepTarget> {
    let mut out = vec![StepTarget::Group];
    if !open {
        return out;
    }
    for (si, step) in steps.iter().enumerate() {
        out.push(StepTarget::Step(si));
        if !step.open || step.calls.is_empty() {
            continue;
        }
        out.push(StepTarget::Calls(si));
        if step.calls_open {
            for ci in 0..step.calls.len() {
                out.push(StepTarget::Call(si, ci));
            }
        }
    }
    out
}

/// A reasoning delta starts a new step after the previous step has already
/// reached calls (or was explicitly sealed). Further deltas of the same
/// reasoning run append to the still-open, call-less step.
fn reasoning_starts_step(steps: &[Step]) -> bool {
    match steps.last() {
        Some(s) => !s.calls.is_empty() || s.sealed,
        None => true,
    }
}

/// Append `call` to the current reasoning-owned step. Calls never create a
/// boundary themselves: sequential and parallel calls both accumulate until
/// a later reasoning run opens the next step. `thinking` is a compatibility
/// input for callers folding an old top-level Thinking block.
fn append_step_call(steps: &mut Vec<Step>, mut thinking: Vec<Line<'static>>, call: ToolCall) {
    let thinking_raw = span_text(&thinking);
    if steps.is_empty() || (!thinking.is_empty() && reasoning_starts_step(steps)) {
        steps.push(Step {
            thinking_raw,
            thinking,
            thinking_dirty: false,
            calls: vec![call],
            open: false,
            calls_open: false,
            sealed: true,
        });
        return;
    }
    if let Some(s) = steps.last_mut() {
        if !thinking.is_empty() {
            s.thinking_raw.push_str(&thinking_raw);
            if !s.thinking_dirty {
                s.thinking.append(&mut thinking);
            }
        }
        s.calls.push(call);
    }
}

/// A fresh single-step group carrying `call` (live `ToolStart` with no
/// trailing group, replayed group seeds, orphan synthesis). Starts fully
/// collapsed (turn + step + calls list + call result). The group is settled
/// by default; live callers explicitly activate progress when appropriate.
pub(crate) fn single_step_group(call: ToolCall, thinking: Vec<Line<'static>>) -> ChatBlock {
    let thinking_raw = span_text(&thinking);
    ChatBlock::StepGroup {
        steps: vec![Step {
            thinking_raw,
            thinking,
            thinking_dirty: false,
            calls: vec![call],
            open: false,
            calls_open: false,
            sealed: true,
        }],
        open: false,
        progress_active: false,
    }
}

/// Set the progress animation on the canonical StepGroup owned by the
/// current admitted user turn. Presentation blocks inside the turn are
/// ignored; the first group after `turn_start` is the single source of truth.
pub(crate) fn set_turn_progress(blocks: &mut [ChatBlock], turn_start: usize, active: bool) -> bool {
    let floor = turn_start.min(blocks.len());
    // Say is the terminal presentation of a Step. Once non-empty assistant
    // output exists in this admitted turn, later reasoning/tool frames must
    // not re-arm the progress indicator on the already-finished ladder.
    let active = active && !turn_has_say(&blocks[floor..]);
    let Some(group) = blocks[floor..]
        .iter_mut()
        .find(|block| matches!(block, ChatBlock::StepGroup { .. }))
    else {
        return false;
    };
    let ChatBlock::StepGroup {
        progress_active, ..
    } = group
    else {
        unreachable!("step-group was matched above");
    };
    *progress_active = active;
    true
}

fn turn_has_say(blocks: &[ChatBlock]) -> bool {
    blocks
        .iter()
        .any(|block| matches!(block, ChatBlock::Assistant { raw, .. } if !raw.is_empty()))
}

/// Concatenate already-rendered spans as a plain-text fallback for replayed
/// or one-shot step construction. Live streaming never round-trips through
/// this lossy representation; it accumulates `Step::thinking_raw` instead.
pub(crate) fn span_text(lines: &[Line<'static>]) -> String {
    let mut out = String::new();
    for line in lines {
        for span in &line.spans {
            out.push_str(&span.content);
        }
    }
    out
}

/// Stream one reasoning delta straight into the ladder — thinking is a
/// structural part of a step, never a top-level block. The first delta after
/// one or more calls opens a new step; later deltas append there, and every
/// call before the next reasoning run remains in the preceding step. A fresh
/// group is pushed when the turn has no ladder yet. Streaming updates never change disclosure state:
/// the turn and step stay
/// closed by default, so the stable top-level view remains `N Steps + Say`.
///
/// An open streaming `Assistant` (the round's Say) rides on top of the
/// ladder: the caller pops it before calling and pushes it back after, so
/// the delta always addresses the group underneath.
pub(crate) fn append_step_thinking_delta(
    blocks: &mut Vec<ChatBlock>,
    turn_start: usize,
    delta: &str,
) -> Option<usize> {
    if delta.is_empty() {
        return None;
    }
    let floor = turn_start.min(blocks.len());
    let should_show_progress = !turn_has_say(&blocks[floor..]);
    let found = blocks[floor..]
        .iter()
        .position(|block| matches!(block, ChatBlock::StepGroup { .. }))
        .map(|relative| floor + relative);
    let (group_idx, inserted_at) = match found {
        Some(idx) => (idx, None),
        None => {
            blocks.insert(
                floor,
                ChatBlock::StepGroup {
                    steps: Vec::new(),
                    open: false,
                    progress_active: should_show_progress,
                },
            );
            (floor, Some(floor))
        }
    };
    if let ChatBlock::StepGroup {
        steps,
        open,
        progress_active,
    } = &mut blocks[group_idx]
    {
        *progress_active = should_show_progress;
        if reasoning_starts_step(steps) {
            steps.push(Step {
                thinking_raw: delta.to_string(),
                thinking: Vec::new(),
                thinking_dirty: true,
                calls: Vec::new(),
                open: false,
                calls_open: false,
                sealed: false,
            });
        } else if let Some(step) = steps.last_mut() {
            step.thinking_raw.push_str(delta);
            if *open && step.open {
                render_step_thinking(step);
            } else {
                step.thinking_dirty = true;
            }
        }
    }
    inserted_at
}

/// Add a call to the one canonical ladder owned by the current user turn.
/// `turn_start` is the live boundary set by `ChatView::begin_turn`; Say,
/// image, marker, and subagent blocks inside that turn never manufacture a
/// second top-level group. A newly-created group is inserted at the boundary
/// so the settled order is always `N Steps` followed by the turn's Say.
pub(crate) fn merge_turn_call(
    blocks: &mut Vec<ChatBlock>,
    turn_start: usize,
    thinking: Vec<Line<'static>>,
    call: ToolCall,
) -> Option<usize> {
    let floor = turn_start.min(blocks.len());
    if let Some(idx) = blocks[floor..]
        .iter()
        .position(|block| matches!(block, ChatBlock::StepGroup { .. }))
        .map(|relative| floor + relative)
    {
        if let ChatBlock::StepGroup { steps, .. } = &mut blocks[idx] {
            append_step_call(steps, thinking, call);
        }
        return None;
    }
    blocks.insert(floor, single_step_group(call, thinking));
    Some(floor)
}

/// Materialize accumulated reasoning only when a step is visible.
pub(crate) fn render_step_thinking(step: &mut Step) {
    if step.thinking_dirty {
        step.thinking = crate::markdown::render(&step.thinking_raw);
        step.thinking_dirty = false;
    }
}

/// A fresh single-step group holding only `thinking` (no function calls) —
/// the ladder shape for a pure-text round at flush time. Starts fully
/// collapsed (group + step), mirroring `single_step_group`.
pub(crate) fn thinking_step_group(thinking: Vec<Line<'static>>) -> ChatBlock {
    let thinking_raw = span_text(&thinking);
    ChatBlock::StepGroup {
        steps: vec![Step {
            thinking_raw,
            thinking,
            thinking_dirty: false,
            calls: Vec::new(),
            open: false,
            calls_open: false,
            sealed: true,
        }],
        open: false,
        progress_active: false,
    }
}

/// Place already-rendered `thinking` into the step ladder at a point where no
/// tool call will consume it (run end, or a boundary push). Walking backwards
/// over the trailing run of `Assistant` blocks — the turn's own speech, still
/// transparent exactly like the absorb walk — a trailing `StepGroup` of this
/// turn gains the thinking as a call-less step; any other block (User echo,
/// Marker, Subagent, ...) caps the run, and a fresh single-step group is
/// inserted before the speech (or the boundary itself when no speech
/// trails). Returns the insert position when a NEW group was inserted, so
/// callers can keep block-index bookkeeping (e.g. `hidden_assistant_idx`)
/// consistent. Pure w.r.t. everything but `blocks`.
pub(crate) fn place_thinking_step(
    blocks: &mut Vec<ChatBlock>,
    thinking: Vec<Line<'static>>,
) -> Option<usize> {
    if thinking.is_empty() {
        return None;
    }
    let mut insert_at = blocks.len();
    for i in (0..blocks.len()).rev() {
        match &mut blocks[i] {
            ChatBlock::Assistant { .. } => insert_at = i,
            ChatBlock::StepGroup { steps, .. } => {
                steps.push(Step {
                    thinking_raw: span_text(&thinking),
                    thinking,
                    thinking_dirty: false,
                    calls: Vec::new(),
                    open: false,
                    calls_open: false,
                    sealed: true,
                });
                return None;
            }
            _ => {
                // The thinking streamed after this boundary (it was the
                // absorb walk's stop block) — the ladder belongs to the
                // current segment, i.e. right AFTER the boundary block.
                insert_at = i + 1;
                break;
            }
        }
    }
    blocks.insert(insert_at, thinking_step_group(thinking));
    Some(insert_at)
}

/// Flush the pending `Thinking` run into the ladder: `absorb_pending_thinking`
/// collects + removes the trailing blocks (Assistant-transparent, boundary-
/// capped), then [`place_thinking_step`] files them as a call-less step.
/// Called at every push where no `ToolStart` can follow — run end (Done /
/// Error), user-echo, subagent, marker, compaction, `!cmd` — so the invariant
/// "thinking exists only at the tail or inside the ladder" holds after every
/// event.
pub(crate) fn flush_pending_thinking(blocks: &mut Vec<ChatBlock>) -> Option<usize> {
    let thinking = absorb_pending_thinking(blocks);
    place_thinking_step(blocks, thinking)
}

/// Absorb the round's pending `Thinking` blocks from the tail of `blocks`,
/// returning their rendered markdown in transcript order and removing the
/// blocks. Walking backwards, `Assistant` blocks are transparent — they are
/// the turn's own speech (intermediate remarks or the final Say), never a
/// round boundary — so reasoning streamed before interim text still folds
/// into the step the next `ToolStart` opens. Any other block type (User,
/// Marker, StepGroup, Subagent, Compaction, Image, Plan) is a boundary: the
/// walk stops there, so a previous user segment's thinking is never
/// absorbed. Pure w.r.t. everything but the argument.
pub(crate) fn absorb_pending_thinking(blocks: &mut Vec<ChatBlock>) -> Vec<Line<'static>> {
    // `drop_idx` is collected in descending index order (backwards walk).
    let mut texts: Vec<String> = Vec::new();
    let mut drop_idx: Vec<usize> = Vec::new();
    for (i, block) in blocks.iter().enumerate().rev() {
        match block {
            ChatBlock::Assistant { .. } => continue,
            ChatBlock::Thinking { text, .. } => {
                texts.push(text.clone());
                drop_idx.push(i);
            }
            _ => break,
        }
    }
    if drop_idx.is_empty() {
        return Vec::new();
    }
    // The backwards walk collected descending indices; the rebuild below
    // walks ascending, so reverse into ascending order first.
    drop_idx.reverse();
    let mut drop_iter = drop_idx.into_iter().peekable();
    let mut kept: Vec<ChatBlock> = Vec::with_capacity(blocks.len() - drop_iter.len());
    for (i, block) in blocks.drain(..).enumerate() {
        if drop_iter.peek() == Some(&i) {
            drop_iter.next();
        } else {
            kept.push(block);
        }
    }
    *blocks = kept;
    // Undo the backwards walk, then render each block's markdown in order.
    let mut thinking = Vec::new();
    for text in texts.into_iter().rev() {
        let mut rendered = crate::markdown::render(&text);
        thinking.append(&mut rendered);
    }
    thinking
}

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
                        ..
                    }) => {
                        prev.append(&mut steps);
                        *prev_progress |= progress_active;
                    }
                    _ => out.push(ChatBlock::StepGroup {
                        steps,
                        open,
                        progress_active,
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

/// Canonicalize replay into one StepGroup per user turn. Persisted sessions
/// are message-pair streams, so Assistant Say and tool-result carrier rows
/// can sit between tool rounds; those are presentation details, not Turn
/// boundaries. The real user echo is the boundary. The merged group is
/// placed before the turn's first Assistant speech, matching the live path's
/// `turn_block_start` insertion and the default `N Steps + Say` order.
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
        for block in segment.iter_mut() {
            if let ChatBlock::StepGroup {
                steps: group_steps,
                open: group_open,
                progress_active: group_progress,
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
                            continue;
                        }
                    }
                    steps.push(step);
                }
                open |= *group_open;
                progress_active |= *group_progress;
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
            });
        }
    }

    let mut out = Vec::with_capacity(blocks.len());
    let mut segment = Vec::new();
    for block in blocks.drain(..) {
        if matches!(block, ChatBlock::User { .. }) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_targets_mirror_the_three_level_ladder() {
        let step = |open: bool, calls_open: bool, calls: usize| Step {
            thinking_raw: String::new(),
            thinking: Vec::new(),
            thinking_dirty: false,
            calls: (0..calls)
                .map(|i| ToolCall {
                    id: format!("c{i}"),
                    header: Line::from("h"),
                    output: Vec::new(),
                    started_at_ms: None,
                    elapsed_ms: None,
                    expanded: false,
                })
                .collect(),
            open,
            calls_open,
            sealed: true,
        };
        let steps = vec![
            step(false, false, 2),
            step(true, false, 1),
            step(true, true, 1),
        ];
        // Group closed: only the group row.
        assert_eq!(visible_targets(false, &steps), vec![StepTarget::Group]);
        // Turn open: every step row is present; open steps add the calls
        // aggregation row, and only a calls-open step adds its call rows.
        assert_eq!(
            visible_targets(true, &steps),
            vec![
                StepTarget::Group,
                StepTarget::Step(0),
                StepTarget::Step(1),
                StepTarget::Calls(1),
                StepTarget::Step(2),
                StepTarget::Calls(2),
                StepTarget::Call(2, 0),
            ]
        );
    }

    #[test]
    fn absorb_pending_thinking_walks_back_over_assistant_blocks() {
        let mk = |blocks: Vec<ChatBlock>| blocks;
        let mut blocks = mk(vec![
            ChatBlock::User {
                rendered: Vec::new(),
            },
            ChatBlock::Thinking {
                text: "first".into(),
                collapsed: true,
                sealed: true,
            },
            ChatBlock::Assistant {
                raw: String::new(),
                rendered: Vec::new(),
                done: true,
            },
            ChatBlock::Thinking {
                text: "second".into(),
                collapsed: true,
                sealed: true,
            },
            ChatBlock::Assistant {
                raw: "say".into(),
                rendered: Vec::new(),
                done: true,
            },
        ]);
        let thinking = absorb_pending_thinking(&mut blocks);
        let joined: String = thinking
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.clone())
            .collect();
        assert!(
            joined.contains("first") && joined.contains("second"),
            "both thinking blocks absorbed in order: {joined:?}"
        );
        // The Assistant blocks survive; the Thinking blocks are gone.
        assert_eq!(blocks.len(), 3);
        assert!(matches!(blocks[1], ChatBlock::Assistant { .. }));
        assert!(!blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Thinking { .. })));
    }

    #[test]
    fn absorb_pending_thinking_stops_at_the_user_boundary() {
        // A User block is an opaque boundary: thinking from a PREVIOUS turn
        // (before the prompt) must never leak into the next turn's step.
        let mut blocks = vec![
            ChatBlock::Thinking {
                text: "previous turn".into(),
                collapsed: true,
                sealed: true,
            },
            ChatBlock::User {
                rendered: Vec::new(),
            },
            ChatBlock::Assistant {
                raw: "say".into(),
                rendered: Vec::new(),
                done: true,
            },
        ];
        assert!(absorb_pending_thinking(&mut blocks).is_empty());
        assert_eq!(blocks.len(), 3, "nothing behind a User block is touched");
    }
}
