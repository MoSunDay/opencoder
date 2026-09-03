//! Shared step semantics for `ChatBlock::StepGroup` — one implementation used
//! by BOTH the live streaming path (`ChatView::apply`) and replay
//! (`session_ui::replay`), so step boundaries never drift between them.
//!
//! A step is one assistant round: its thinking plus that round's function
//! calls. Boundary heuristic: a new `ToolStart` merges into the trailing
//! step while it still holds no finished call, and opens a NEW step once it
//! does (sequential calls in one round thereby split; parallel calls stay
//! together).

use ratatui::text::Line;

use super::{ChatBlock, Step, ToolCall};

/// One clickable row inside a `StepGroup`: a step row (toggles the
/// step) or a call header row (toggles that single call's output).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StepTarget {
    Step(usize),
    Call(usize, usize),
}

/// The group's currently rendered click targets, in visual order: step row
/// first, then (while the step is open) each call's header row. Mirrors the
/// `collect_headers` walk exactly, so `ToolCallHeader::call_idx` indexes this
/// list and `toggle_tool_call_at` resolves the same row the renderer drew.
pub(crate) fn visible_targets(steps: &[Step]) -> Vec<StepTarget> {
    let mut out = Vec::new();
    for (si, step) in steps.iter().enumerate() {
        out.push(StepTarget::Step(si));
        if step.open {
            for (ci, _) in step.calls.iter().enumerate() {
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
/// merging, to the step it joins (kept lossless: extended, never dropped).
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
/// trailing group, replayed group seeds, orphan synthesis).
pub(crate) fn single_step_group(call: ToolCall, thinking: Vec<Line<'static>>) -> ChatBlock {
    ChatBlock::StepGroup {
        steps: vec![Step {
            thinking,
            calls: vec![call],
            open: false,
        }],
    }
}

/// Pop the trailing run of consecutive `Thinking` blocks, returning their
/// rendered markdown as the next step's thinking. Only blocks that TRAIL are
/// absorbed — a round that streamed answer text keeps its `Thinking` block
/// standalone (the assistant block trails, not the thinking). Called after
/// `finalize_assistant`, so every popped block is already sealed and its
/// tokens counted.
pub(crate) fn pop_trailing_thinking(blocks: &mut Vec<ChatBlock>) -> Vec<Line<'static>> {
    let mut thinking = Vec::new();
    while let Some(ChatBlock::Thinking { text, .. }) = blocks.last() {
        let text = text.clone();
        blocks.pop();
        let mut rendered = crate::markdown::render(&text);
        thinking.append(&mut rendered);
    }
    thinking
}

/// Fold replayed blocks: absorb each run of trailing `Thinking` blocks into
/// the following `StepGroup`'s first step, and merge runs of adjacent
/// `StepGroup`s into one group. Standalone `Thinking` blocks (not followed by
/// a group) keep their own rendering path. Pure w.r.t. everything but the
/// argument.
pub(crate) fn coalesce_steps(blocks: &mut Vec<ChatBlock>) {
    let mut out: Vec<ChatBlock> = Vec::with_capacity(blocks.len());
    let mut pending: Vec<ChatBlock> = Vec::new();
    for block in blocks.drain(..) {
        match block {
            ChatBlock::Thinking { .. } => pending.push(block),
            ChatBlock::StepGroup { mut steps } => {
                if !pending.is_empty() {
                    if let Some(first) = steps.first_mut() {
                        let mut absorbed = take_rendered_thinking(&mut pending);
                        first.thinking.append(&mut absorbed);
                    }
                }
                // Drain anything left (group with zero steps — defensive):
                // the thinking stays standalone.
                if !steps.is_empty() {
                    match out.last_mut() {
                        Some(ChatBlock::StepGroup { steps: prev, .. }) => prev.append(&mut steps),
                        _ => out.push(ChatBlock::StepGroup { steps }),
                    }
                }
                out.append(&mut pending);
            }
            other => {
                out.append(&mut pending);
                out.push(other);
            }
        }
    }
    out.append(&mut pending);
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
