//! Header line-accounting for Thinking / Subagent / Tool / Compaction blocks.
//!
//! Walks `ChatView.blocks` with the identical per-block line accounting that
//! `flatten_with` emits, so click hit-rects stay aligned with the live render.
//! Extracted from `chat.rs` for the line gate; behavior unchanged.

use super::{
    assistant_rows, ChatBlock, ChatView, CompactionHeader, SubagentHeader, ThinkingHeader,
    ToolHeader,
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

    /// Single pass over all blocks computing the header line index of every
    /// Thinking / Subagent / Tool block, using the identical per-block line
    /// accounting that `flatten_with()` emits. Keeping the accounting in one
    /// place guarantees hit-rect indices stay aligned with the live render.
    pub(crate) fn collect_headers(
        &self,
    ) -> (
        Vec<ThinkingHeader>,
        Vec<SubagentHeader>,
        Vec<ToolHeader>,
        Vec<CompactionHeader>,
    ) {
        let mut think = Vec::new();
        let mut sub = Vec::new();
        let mut tool = Vec::new();
        let mut compaction = Vec::new();
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
                ChatBlock::Tool {
                    output, collapsed, ..
                } => {
                    tool.push(ToolHeader {
                        block_idx,
                        header_line_idx: line_idx,
                    });
                    // Header always rendered; output + trailing blank only when expanded.
                    line_idx += 1 + if *collapsed { 0 } else { output.len() + 1 };
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
                ChatBlock::Plan { rendered, .. } => {
                    line_idx += 1 + rendered.len() + 1;
                }
            }
        }
        (think, sub, tool, compaction)
    }
}
