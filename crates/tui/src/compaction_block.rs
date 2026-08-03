//! Collapsible compaction-summary block: shared rendering + state helpers.
//! Extracted from `chat.rs` to keep that file under the size limit. Mirrors
//! the Thinking block — muted italic styling, default collapsed, click-to-
//! expand — so compaction output is visually consistent with reasoning.

use crate::chat::{ChatBlock, ChatView, CompactionHeader};
use crate::theme;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// Render a collapsible text block (used by both Thinking and Compaction).
///
/// When collapsed: a single muted header line showing the icon + label and a
/// `(N lines)` count summarizing how many lines are hidden.
/// When expanded: a muted italic-bold header followed by each line indented
/// 2 spaces in muted italic. Click-to-expand is wired separately via the
/// hit-rect pipeline.
pub(crate) fn render_collapsible(
    icon: &str,
    label: &str,
    text: &str,
    collapsed: bool,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    if collapsed {
        let n = text.lines().count();
        out.push(Line::from(Span::styled(
            format!("{icon} {label} ({n} lines)"),
            Style::default().fg(theme::muted()),
        )));
    } else {
        out.push(Line::from(Span::styled(
            format!("{icon} {label}"),
            Style::default()
                .fg(theme::muted())
                .add_modifier(Modifier::ITALIC | Modifier::BOLD),
        )));
        for l in text.lines() {
            out.push(Line::from(Span::styled(
                format!("  {l}"),
                Style::default()
                    .fg(theme::muted())
                    .add_modifier(Modifier::ITALIC),
            )));
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
