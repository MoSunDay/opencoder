//! Render viewport virtualization.
//!
//! Flattening a `ChatView` into `Line`s and counting wrapped screen rows is an
//! O(n) operation proportional to transcript length. Doing it on every
//! animation frame (the spinner tick) makes per-frame cost grow with the
//! conversation. This module caches the flattened lines plus a cumulative
//! row-offset table so the visible window can be located in O(log n) via binary
//! search and only those lines cloned for the `Paragraph` widget.

use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};

/// Number of screen rows `line` occupies when word-wrapped at width `w`,
/// matching ratatui's `Paragraph` wrapping exactly. An empty line is 1 row.
pub(crate) fn wrapped_rows(line: &Line<'_>, w: u16) -> usize {
    Paragraph::new(line.clone())
        .wrap(Wrap { trim: false })
        .line_count(w)
}

/// Cached flattened content of a ChatView for rendering. Computed once per
/// body-refresh cycle (or on width change), reused every frame to avoid
/// re-flattening and re-counting wrapped rows for the entire transcript.
///
/// Stores the full flattened `Vec<Line>` plus a cumulative row-offset table
/// (`cum_rows`) so `render_body` can binary-search the visible window in
/// O(log n) and clone only the visible lines for the Paragraph widget.
#[derive(Clone)]
pub struct ViewportCache {
    lines: Vec<Line<'static>>,
    /// `cum_rows[i]` = total screen rows consumed by `lines[0..i]`.
    /// `cum_rows.len() == lines.len() + 1`; `cum_rows[0] == 0`;
    /// `total_rows == *cum_rows.last().unwrap()`.
    cum_rows: Vec<usize>,
    /// Total wrapped screen rows across all lines.
    total_rows: usize,
    /// Terminal text width the layout was computed at. If the terminal is
    /// resized the cache must be rebuilt.
    width: u16,
}

impl ViewportCache {
    /// Build a cache for `chat` at `width`, advancing the spinner via
    /// `anim_tick`. Flattening and wrapped-row counting happen exactly once
    /// here; subsequent frames consult [`Self::visible_window`].
    ///
    /// A `width` of 0 (e.g. terminal too narrow) yields an empty cache so
    /// callers can short-circuit without dividing by zero.
    pub fn build(chat: &crate::chat::ChatView, width: u16, anim_tick: u32, now_ms: i64) -> Self {
        if width == 0 {
            return ViewportCache {
                lines: Vec::new(),
                // Preserve the invariant cum_rows.len() == lines.len() + 1.
                cum_rows: vec![0],
                total_rows: 0,
                width: 0,
            };
        }
        let lines = chat.flatten_with(anim_tick, now_ms);
        // cum_rows[i] = rows consumed by lines[0..i]; cum_rows[0] == 0.
        let mut cum_rows = Vec::with_capacity(lines.len() + 1);
        cum_rows.push(0);
        let mut acc = 0usize;
        for line in &lines {
            acc += wrapped_rows(line, width);
            cum_rows.push(acc);
        }
        let total_rows = acc;
        ViewportCache {
            lines,
            cum_rows,
            total_rows,
            width,
        }
    }

    /// Total wrapped screen rows across all lines (`*cum_rows.last()`).
    pub fn total_rows(&self) -> usize {
        self.total_rows
    }

    /// Terminal text width the layout was computed at.
    pub fn width(&self) -> u16 {
        self.width
    }

    /// Borrow the flattened lines.
    pub fn lines(&self) -> &[Line<'static>] {
        &self.lines
    }

    /// Screen row where logical line `line_idx` starts (`cum_rows[line_idx]`).
    /// Returns `total_rows` for out-of-range indices so callers can safely
    /// compare against bounds.
    pub fn row_of_line(&self, line_idx: usize) -> usize {
        self.cum_rows
            .get(line_idx)
            .copied()
            .unwrap_or(self.total_rows)
    }

    /// Locate the visible logical-line window for a viewport scrolled to
    /// `scroll_y` with `visible_h` screen rows.
    ///
    /// Returns `(start_line, end_line_exclusive, top_skip_rows)`:
    /// - `start_line` — first visible logical line index.
    /// - `end_line` — one past the last visible logical line index.
    /// - `top_skip` — screen rows of the first visible line to skip before
    ///   the viewport top (`scroll_y - cum_rows[start_line]`).
    ///
    /// Found via two binary searches over `cum_rows`, so it is O(log n)
    /// regardless of transcript length. Empty input or `visible_h == 0`
    /// yields `(0, 0, 0)`.
    pub fn visible_window(&self, scroll_y: usize, visible_h: usize) -> (usize, usize, usize) {
        if self.lines.is_empty() || visible_h == 0 {
            return (0, 0, 0);
        }
        // Start: first line whose end row (`cum_rows[i+1]`) exceeds scroll_y.
        // cum_rows[1..][j] == cum_rows[j+1]; the matching line index is j.
        let mut start = self.cum_rows[1..].partition_point(|&r| r <= scroll_y);
        let last_line = self.lines.len() - 1;
        if start > last_line {
            start = last_line;
        }
        let top_skip = scroll_y.saturating_sub(self.cum_rows[start]);
        // End: first line whose start row (`cum_rows[i]`) reaches `bottom`.
        let bottom = scroll_y.saturating_add(visible_h);
        let mut end = self.cum_rows.partition_point(|&r| r < bottom);
        if end > self.lines.len() {
            end = self.lines.len();
        }
        // Always expose at least the start line so render_body gets a slice.
        if end <= start {
            end = start + 1;
        }
        (start, end, top_skip)
    }
}
