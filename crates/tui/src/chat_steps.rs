//! Shared step semantics for `ChatBlock::StepGroup` — one implementation used
//! by BOTH the live streaming path (`ChatView::apply`) and replay
//! (`session_ui::replay`), so step boundaries never drift between them.
//!
//! A step is one assistant round: its thinking plus that round's function
//! calls. Boundary heuristic: a new `ToolStart` merges into the trailing
//! step while it still holds no finished call, and opens a NEW step once it
//! does (sequential calls in one round thereby split; parallel calls stay
//! together).
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

/// One clickable row inside a `StepGroup`'s three-level ladder: the group
/// row (toggles the whole group), a step row (toggles that step), a calls
/// aggregation row (toggles that step's call list), or a call header row
/// (toggles that single call's output).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StepTarget {
    Group,
    Step(usize),
    Calls(usize),
    Call(usize, usize),
}

/// The group's currently rendered click targets, in visual order: the group
/// row always; while the group is open each step row; while a step is open
/// (and holds calls) its aggregation row; while the call list is open each
/// call header row. Mirrors the `collect_headers` walk exactly, so
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

/// Step-boundary judgment shared by the live and replay paths: `true` when a
/// new call must NOT merge into the trailing step (it already holds a
/// finished call), i.e. a new step opens.
fn boundary_needed(steps: &[Step]) -> bool {
    match steps.last() {
        Some(s) => s.calls.iter().any(|c| c.elapsed_ms.is_some()),
        None => true,
    }
}

/// Append `call` to `steps` honoring the shared boundary heuristic, attaching
/// `thinking` (rendered markdown lines) to the step it opens — or, when
/// merging, to the trailing step. Every level of the ladder starts closed.
pub(crate) fn merge_or_new_step(
    steps: &mut Vec<Step>,
    mut thinking: Vec<Line<'static>>,
    call: ToolCall,
) {
    if boundary_needed(steps) {
        steps.push(Step {
            thinking,
            calls: vec![call],
            open: false,
            calls_open: false,
        });
        return;
    }
    if let Some(s) = steps.last_mut() {
        if !thinking.is_empty() {
            s.thinking.append(&mut thinking);
        }
        s.calls.push(call);
    }
}

/// A fresh single-step group carrying `call` (live `ToolStart` with no
/// trailing group, replayed group seeds, orphan synthesis). Starts fully
/// collapsed (group + step + calls list + call output).
pub(crate) fn single_step_group(call: ToolCall, thinking: Vec<Line<'static>>) -> ChatBlock {
    ChatBlock::StepGroup {
        steps: vec![Step {
            thinking,
            calls: vec![call],
            open: false,
            calls_open: false,
        }],
        open: false,
    }
}

/// A fresh single-step group holding only `thinking` (no function calls) —
/// the ladder shape for a pure-text round at flush time. Starts fully
/// collapsed (group + step), mirroring `single_step_group`.
pub(crate) fn thinking_step_group(thinking: Vec<Line<'static>>) -> ChatBlock {
    ChatBlock::StepGroup {
        steps: vec![Step {
            thinking,
            calls: Vec::new(),
            open: false,
            calls_open: false,
        }],
        open: false,
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
                    thinking,
                    calls: Vec::new(),
                    open: false,
                    calls_open: false,
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
            ChatBlock::StepGroup { mut steps, open } => {
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
                steps[0].thinking.append(&mut absorbed);
                match out.last_mut() {
                    Some(ChatBlock::StepGroup { steps: prev, .. }) => prev.append(&mut steps),
                    _ => out.push(ChatBlock::StepGroup { steps, open }),
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
            thinking: Vec::new(),
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
        };
        let steps = vec![
            step(false, false, 2),
            step(true, false, 1),
            step(true, true, 1),
        ];
        // Group closed: only the group row.
        assert_eq!(visible_targets(false, &steps), vec![StepTarget::Group]);
        // Group open: step rows; open steps add the aggregation row; only a
        // calls_open step adds its call rows.
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
