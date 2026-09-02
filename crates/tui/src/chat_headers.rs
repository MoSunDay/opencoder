//! Header line-accounting for Thinking / Subagent / Tool / Compaction blocks.
//!
//! Walks `ChatView.blocks` with the identical per-block line accounting that
//! `flatten_with` emits, so click hit-rects stay aligned with the live render.
//! Extracted from `chat.rs` for the line gate; behavior unchanged.

use super::{
    assistant_rows, ChatBlock, ChatView, CompactionHeader, SubagentHeader, ThinkingHeader,
    ToolCallHeader, ToolHeader,
};

impl ChatView {
    /// Return each Thinking block's `(block_idx, header_line_idx)`, where
    /// `header_line_idx` is the index in `flatten()` of its header line. Walks
    /// the blocks with the same per-block line accounting `flatten()` uses, so
    /// the indices stay in sync with what is rendered. Used by `render_body`
    /// to build click hit-rects.
    pub fn thinking_headers(&self) -> Vec<ThinkingHeader> {
        self.collect_headers().0
    }

    /// Return each Subagent block's `(block_idx, header_line_idx)`. Mirrors
    /// `thinking_headers`; used to build click hit-rects for subagent headers.
    pub fn subagent_headers(&self) -> Vec<SubagentHeader> {
        self.collect_headers().1
    }

    /// Return each Tool block's `(block_idx, header_line_idx)`. Mirrors
    /// `thinking_headers`; used to build click hit-rects for tool headers.
    pub fn tool_headers(&self) -> Vec<ToolHeader> {
        self.collect_headers().2
    }

    /// Return each clickable row inside open `StepGroup` blocks — step rows
    /// and (while a step is open) its call header rows, in render order — as
    /// `(block_idx, call_idx, header_line_idx)`. `call_idx` is the FLAT index
    /// into that visible-row walk (shared shape with
    /// `super::steps::visible_targets`), so clicking resolves exactly the
    /// rendered row.
    pub fn tool_call_headers(&self) -> Vec<ToolCallHeader> {
        self.collect_headers().4
    }

    /// Single pass over all blocks computing the header line index of every
    /// Thinking / Subagent / Tool block, using the identical per-block line
    /// accounting that `flatten_with()` emits. Keeping the accounting in one
    /// place guarantees hit-rect indices stay aligned with the live render.
    #[allow(clippy::type_complexity)]
    pub(crate) fn collect_headers(
        &self,
    ) -> (
        Vec<ThinkingHeader>,
        Vec<SubagentHeader>,
        Vec<ToolHeader>,
        Vec<CompactionHeader>,
        Vec<ToolCallHeader>,
    ) {
        let mut think = Vec::new();
        let mut sub = Vec::new();
        let mut tool = Vec::new();
        let mut compaction = Vec::new();
        let mut tool_calls = Vec::new();
        let mut line_idx = 0usize;
        for (block_idx, block) in self.blocks.iter().enumerate() {
            match block {
                ChatBlock::Marker(lines) => line_idx += lines.len(),
                ChatBlock::User { rendered } => line_idx += 1 + rendered.len(),
                ChatBlock::Assistant {
                    raw,
                    rendered,
                    done,
                } => {
                    // Withheld preamble renders zero lines (issue #5); skip it
                    // so header line indices stay aligned with `flatten_with`.
                    if self.is_withheld(block_idx) {
                        continue;
                    }
                    // +1 for the "say:" header line emitted by flatten().
                    line_idx += 1;
                    line_idx += if *done {
                        rendered.len()
                    } else {
                        assistant_rows(raw).len()
                    };
                }
                ChatBlock::Thinking {
                    text, collapsed, ..
                } => {
                    think.push(ThinkingHeader {
                        block_idx,
                        header_line_idx: line_idx,
                    });
                    // Header line always emitted; content lines only when expanded.
                    line_idx += 1;
                    if !collapsed {
                        line_idx += text.lines().count();
                    }
                }
                ChatBlock::StepGroup { steps, open } => {
                    tool.push(ToolHeader {
                        block_idx,
                        header_line_idx: line_idx,
                    });
                    // Mirrors `flatten_step_group` exactly: the group row
                    // always; while the group is open, per step the step row,
                    // and while THAT step is open its `❯ Say:` header +
                    // thinking lines and per call (header + output + blank
                    // when the call is expanded); one trailing blank.
                    line_idx += 1;
                    if *open {
                        let mut call_idx = 0usize;
                        for step in steps.iter() {
                            tool_calls.push(ToolCallHeader {
                                block_idx,
                                call_idx,
                                header_line_idx: line_idx,
                            });
                            call_idx += 1;
                            line_idx += 1; // step row
                            if step.open {
                                if !step.thinking.is_empty() {
                                    line_idx += 1 + step.thinking.len();
                                }
                                for c in &step.calls {
                                    tool_calls.push(ToolCallHeader {
                                        block_idx,
                                        call_idx,
                                        header_line_idx: line_idx,
                                    });
                                    call_idx += 1;
                                    line_idx += 1 + if c.expanded { 1 + c.output.len() } else { 0 };
                                }
                            }
                        }
                        line_idx += 1; // trailing blank
                    }
                }
                ChatBlock::Compaction {
                    text, collapsed, ..
                } => {
                    compaction.push(CompactionHeader {
                        block_idx,
                        header_line_idx: line_idx,
                    });
                    line_idx += 1;
                    if !collapsed {
                        line_idx += text.lines().count();
                    }
                }
                ChatBlock::Image { rendered, .. } => {
                    // Empty `rendered` -> flatten_with emits a placeholder line; count 1 not 0.
                    line_idx +=
                        1 + if rendered.is_empty() {
                            1
                        } else {
                            rendered.len()
                        } + 1;
                }
                ChatBlock::Subagent { .. } => {
                    sub.push(SubagentHeader {
                        block_idx,
                        header_line_idx: line_idx,
                    });
                    line_idx += 1; // header only — no inline expansion
                }
                ChatBlock::Sidecar { .. } => {
                    // Zero lines: the sidecar bypass Q/A never shows in the
                    // flat main transcript (focused body is swapped in by
                    // `compute_display`; `sidecar::purge` removes the block
                    // on exit).
                }
                ChatBlock::Plan { rendered, .. } => {
                    line_idx += 1 + rendered.len() + 1;
                }
            }
        }
        (think, sub, tool, compaction, tool_calls)
    }
}
