//! Collapsible compaction-summary block: shared rendering + state helpers.
//! Extracted from `chat.rs` to keep that file under the size limit. Mirrors
//! the Thinking block — default collapsed, click-to-expand — with per-call
//! style (Thinking uses a dedicated pink header, Compaction
//! stays unstyled).

use crate::chat::{ChatBlock, ChatView, CompactionHeader};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

/// Render a collapsible text block (used by both Thinking and Compaction).
///
/// When collapsed: a single header line showing the icon + label and a
/// `(N lines)` count summarizing how many lines are hidden.
/// When expanded: the header followed by each line indented 2 spaces.
/// `header_style` / `body_style` let callers pick their own palette —
/// Thinking uses a dedicated pink+BOLD header / muted body, Compaction
/// passes `Style::default()` for plain output. Click-to-expand is wired
/// separately via the hit-rect pipeline.
pub(crate) fn render_collapsible(
    icon: &str,
    label: &str,
    text: &str,
    collapsed: bool,
    header_style: Style,
    body_style: Style,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    if collapsed {
        let n = text.lines().count();
        out.push(Line::from(Span::styled(
            format!("{icon} {label} ({n} lines)"),
            header_style,
        )));
    } else {
        out.push(Line::from(Span::styled(
            format!("{icon} {label}"),
            header_style,
        )));
        for l in text.lines() {
            out.push(Line::from(Span::styled(format!("  {l}"), body_style)));
        }
    }
    out
}

impl ChatView {
    /// Toggle collapse on the Compaction block at `block_idx` (mouse click).
    /// No-op if the index is out of range or not a Compaction block.
    pub fn toggle_compaction_at(&mut self, block_idx: usize) {
        if let Some(ChatBlock::Compaction { collapsed, .. }) = self.blocks.get_mut(block_idx) {
            *collapsed = !*collapsed;
        }
    }

    /// Append a streaming compaction delta. If the last block is a still-
    /// streaming Compaction block, append to its text; otherwise finalize any
    /// in-progress assistant block and open a fresh expanded streaming block.
    /// This mirrors `ensure_assistant_open`/`TextDelta` so the summary is
    /// visible while the summarizing LLM call runs, not only after it finishes.
    pub(crate) fn open_compaction_streaming(&mut self, t: &str) {
        self.finalize_assistant();
        if let Some(ChatBlock::Compaction {
            text,
            streaming: true,
            ..
        }) = self.blocks.last_mut()
        {
            text.push_str(t);
            return;
        }
        self.blocks.push(ChatBlock::Compaction {
            text: t.to_string(),
            collapsed: false,
            streaming: true,
        });
    }

    /// Finalize the compaction block with the complete summary. If the last
    /// block is a streaming Compaction, overwrite its text with the final
    /// summary and collapse it; otherwise create a fresh collapsed block (the
    /// streamed block was destroyed, e.g. by a `TranscriptReset` replay).
    pub(crate) fn finalize_compaction(&mut self, summary: &str) {
        self.finalize_assistant();
        if let Some(ChatBlock::Compaction {
            text,
            collapsed,
            streaming,
        }) = self.blocks.last_mut()
        {
            if *streaming {
                *text = summary.to_string();
                *collapsed = true;
                *streaming = false;
                return;
            }
        }
        self.blocks.push(ChatBlock::Compaction {
            text: summary.to_string(),
            collapsed: true,
            streaming: false,
        });
    }

    /// True if the last block is a collapsed Compaction block — per-delta
    /// re-renders can be skipped while the compaction summary streams hidden.
    pub fn last_compaction_collapsed(&self) -> bool {
        matches!(
            self.blocks.last(),
            Some(ChatBlock::Compaction {
                collapsed: true,
                ..
            })
        )
    }

    /// Return each Compaction block's `(block_idx, header_line_idx)`, where
    /// `header_line_idx` is the index in `flatten()` of its header line.
    pub fn compaction_headers(&self) -> Vec<CompactionHeader> {
        self.collect_headers().3
    }
}
