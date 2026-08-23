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
/// The lazily-built [`CleanModel`] (`cleaned`) is the same virtualization
/// over the *decoration-stripped* line set used by copy mode; replacing the
/// cache with a fresh object drops it along with the decorated tables.
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
    /// Copy-mode view over `lines`, built on first [`Self::cleaned`] call at
    /// a given width and rebuilt when that width changes.
    clean: Option<CleanModel>,
}

/// Decoration-stripped projection of a [`ViewportCache`]'s lines for copy
/// mode: every line kept by `copy_mode::clean::clean_line` with its slots
/// already stripped, plus a wrapped-row table with the same semantics as
/// `ViewportCache`'s (`cum_rows[i]` = wrapped rows of `texts[0..i]`). Window
/// math on this model can never land on a dropped row, so scrolling geometry
/// and rendering agree by construction.
#[derive(Clone)]
pub(crate) struct CleanModel {
    texts: Vec<String>,
    /// `cum_rows.len() == texts.len() + 1`; `cum_rows[0] == 0`.
    cum_rows: Vec<usize>,
    /// Total wrapped screen rows across all kept texts.
    total_rows: usize,
    /// Width the wrapped-row table was computed at.
    width: u16,
}

impl CleanModel {
    /// Project `lines` through the structured cleaner and count wrapped rows
    /// of each kept text at `width` (the same `Paragraph` wrapping the clean
    /// view renders with).
    fn build(lines: &[Line<'static>], width: u16) -> Self {
        let mut texts = Vec::new();
        let mut cum_rows = Vec::with_capacity(lines.len() + 1);
        cum_rows.push(0);
        let mut acc = 0usize;
        for line in lines {
            if let Some(text) = crate::copy_mode::clean::clean_line(line) {
                let wrapped = Line::from(text.clone());
                acc += wrapped_rows(&wrapped, width);
                cum_rows.push(acc);
                texts.push(text);
            }
        }
        CleanModel {
            texts,
            cum_rows,
            total_rows: acc,
            width,
        }
    }

    /// Total wrapped screen rows across all kept texts.
    pub(crate) fn total_rows(&self) -> usize {
        self.total_rows
    }

    /// `cum_rows[i]` = screen rows consumed by `texts[0..i]` (len+1 table);
    /// the copy-mode wrap plan derives its soft-row flags from this.
    pub(crate) fn cum_rows(&self) -> &[usize] {
        &self.cum_rows
    }

    /// The kept, slot-stripped texts (window indices refer to this slice).
    pub(crate) fn texts(&self) -> &[String] {
        &self.texts
    }

    /// Locate the visible kept-text window for a viewport scrolled to
    /// `scroll_y` with `visible_h` screen rows — the same two binary searches
    /// and clamps as `ViewportCache::visible_window`, over the cleaned table.
    ///
    /// Returns `(start, end_exclusive, top_skip_rows)`.
    pub(crate) fn visible_window(
        &self,
        scroll_y: usize,
        visible_h: usize,
    ) -> (usize, usize, usize) {
        if self.texts.is_empty() || visible_h == 0 {
            return (0, 0, 0);
        }
        let mut start = self.cum_rows[1..].partition_point(|&r| r <= scroll_y);
        let last = self.texts.len() - 1;
        if start > last {
            start = last;
        }
        let top_skip = scroll_y.saturating_sub(self.cum_rows[start]);
        let bottom = scroll_y.saturating_add(visible_h);
        let mut end = self.cum_rows.partition_point(|&r| r < bottom);
        if end > self.texts.len() {
            end = self.texts.len();
        }
        if end <= start {
            end = start + 1;
        }
        (start, end, top_skip)
    }
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
                clean: None,
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
            clean: None,
        }
    }

    /// The copy-mode (decoration-stripped) projection of this cache's lines,
    /// built lazily on first call at `width` and rebuilt if the width
    /// changes. Callers render from [`CleanModel::texts`] directly — no
    /// post-hoc filtering — so window math and pixels share one model.
    pub(crate) fn cleaned(&mut self, width: u16) -> &CleanModel {
        if self.clean.as_ref().is_none_or(|c| c.width != width) {
            self.clean = Some(CleanModel::build(&self.lines, width));
        }
        self.clean.as_ref().expect("clean model built above")
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
    /// Index of the logical line that contains absolute screen `row`
    /// (binary search over `cum_rows`). Rows at or past the end clamp to
    /// the last line; an empty cache returns 0. Inverse of
    /// [`Self::row_of_line`] for row spans: `row_of_line(i) <= row <
    /// row_of_line(i + 1)` implies `line_at_row(row) == i`.
    pub fn line_at_row(&self, row: usize) -> usize {
        if self.lines.is_empty() {
            return 0;
        }
        self.cum_rows[1..]
            .partition_point(|&r| r <= row)
            .min(self.lines.len() - 1)
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::ChatView;
    use ratatui::text::{Line, Span};

    /// A view whose flattened lines are exactly `lines` (one Marker block per
    /// line) — markers render verbatim, independent of the markdown renderer.
    fn view_from_lines(lines: &[&str]) -> ChatView {
        let mut v = ChatView::default();
        for &l in lines {
            v.push_marker(Line::from(l.to_string()));
        }
        v
    }

    #[test]
    fn line_at_row_maps_rows_through_wrapped_lines() {
        // Width 10: the 20-char line wraps into 2 rows, others into 1.
        // cum_rows = [0, 2, 3, 4].
        let v = view_from_lines(&["0123456789abcdefghij", "b", "c"]);
        let cache = ViewportCache::build(&v, 10, 0, 0);
        assert_eq!(cache.total_rows(), 4);
        assert_eq!(cache.line_at_row(0), 0);
        assert_eq!(cache.line_at_row(1), 0, "second wrapped row -> same line");
        assert_eq!(cache.line_at_row(2), 1);
        assert_eq!(cache.line_at_row(3), 2);
        // Past-the-end rows clamp to the last line.
        assert_eq!(cache.line_at_row(4), 2);
        assert_eq!(cache.line_at_row(999), 2);
    }

    #[test]
    fn line_at_row_empty_cache_returns_zero() {
        let cache = ViewportCache::build(&ChatView::default(), 10, 0, 0);
        assert_eq!(cache.total_rows(), 0);
        assert_eq!(cache.line_at_row(0), 0);
    }

    // ── CleanModel (copy-mode virtualization) ─────────────────────────────

    /// A view whose flattened lines have exactly the span contents given.
    fn view_from_span_lines(parts: &[&[&str]]) -> ChatView {
        let mut v = ChatView::default();
        for ps in parts {
            v.push_marker(Line::from(
                ps.iter()
                    .map(|p| Span::raw(p.to_string()))
                    .collect::<Vec<_>>(),
            ));
        }
        v
    }

    /// Marker lines mimicking a real assistant message with a fenced block:
    /// header, indented top frame, two code rows, bottom frame, trailing
    /// blank — plus a plain kept row on each side.
    fn mixed_view() -> ChatView {
        view_from_span_lines(&[
            &["keep-a"],
            &["\u{276f} Say:"],
            &["    ", "\u{250c} rust "],
            &["    ", "\u{2502} ", "fn a() {}"],
            &["    ", "\u{2502} ", "---"],
            &["    ", "\u{2514}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"],
            &["    ", ""],
            &["keep-b"],
        ])
    }

    #[test]
    fn cleaned_total_counts_only_kept_rows() {
        let v = mixed_view();
        let mut cache = ViewportCache::build(&v, 40, 0, 0);
        let clean = cache.cleaned(40);
        assert_eq!(
            clean.texts(),
            &[
                "keep-a".to_string(),
                "fn a() {}".to_string(),
                "---".to_string(),
                "".to_string(),
                "keep-b".to_string()
            ],
            "decoration dropped, slots stripped, code payload verbatim"
        );
        assert_eq!(
            clean.total_rows(),
            5,
            "one wrapped row per kept text at width 40"
        );
    }

    #[test]
    fn cleaned_visible_window_maps_wrapped_rows() {
        // Width 10: the 20-char kept text wraps into 2 rows, the dropped
        // decoration between/after it contributes nothing.
        let v = view_from_span_lines(&[
            &["0123456789abcdefghij"],
            &["\u{276f} Say:"],
            &["b"],
            &["    ", "\u{250c} x "],
            &["c"],
        ]);
        let mut cache = ViewportCache::build(&v, 10, 0, 0);
        let clean = cache.cleaned(10);
        assert_eq!(clean.texts().len(), 3);
        assert_eq!(clean.total_rows(), 4);
        // Top of the transcript.
        assert_eq!(clean.visible_window(0, 1), (0, 1, 0));
        // Scrolled into the second wrapped row of the long first text:
        // top_skip is that text's own in-line offset.
        assert_eq!(clean.visible_window(1, 1), (0, 1, 1));
        // Past the long text onto `b`.
        assert_eq!(clean.visible_window(2, 10), (1, 3, 0));
    }

    #[test]
    fn cleaned_top_skip_takes_first_visible_line_own_offset() {
        // A long kept text (wraps to 2 rows at width 10) followed by another;
        // scrolling to its second row reports top_skip == 1 for that line.
        let v = view_from_span_lines(&[&["\u{276f} Say:"], &["0123456789abcdefghij"], &["tail"]]);
        let mut cache = ViewportCache::build(&v, 10, 0, 0);
        let clean = cache.cleaned(10);
        assert_eq!(clean.visible_window(1, 1), (0, 1, 1));
        assert_eq!(clean.visible_window(1, 5), (0, 2, 1));
    }

    #[test]
    fn cleaned_trailing_decoration_leaves_no_blank_band() {
        // The tail of the transcript is dropped decoration plus one
        // structural blank: the cleaned total is exactly the kept rows, so a
        // bottom-pinned window covers real content instead of scrolling past
        // it into emptiness (the old decorated-geometry blank band).
        let v = view_from_span_lines(&[
            &["only-content"],
            &["    ", "\u{2514}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"],
            &["    ", "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"],
            &["    ", ""],
            &["\u{276f} Say:"],
        ]);
        let mut cache = ViewportCache::build(&v, 40, 0, 0);
        let clean = cache.cleaned(40);
        assert_eq!(clean.texts().len(), 2, "kept: content + structural blank");
        assert_eq!(
            clean.total_rows(),
            2,
            "no phantom rows from the dropped tail"
        );
        // Bottom-pinned window (scroll = total - h clamps to 0) covers both.
        let (start, end, top_skip) = clean.visible_window(0, 8);
        assert_eq!((start, end, top_skip), (0, 2, 0));
    }

    #[test]
    fn cleaned_is_cached_and_rebuilt_on_width_change() {
        // The model is cached per width (no per-frame rebuild) and its
        // wrapped-row table follows the requested width — verified through
        // the observable totals rather than object identity.
        let v = view_from_lines(&["0123456789abcdefghij"]);
        let mut cache = ViewportCache::build(&v, 40, 0, 0);
        assert_eq!(cache.cleaned(40).total_rows(), 1);
        assert_eq!(
            cache.cleaned(40).total_rows(),
            1,
            "same width reuses the cached model"
        );
        assert_eq!(
            cache.cleaned(10).total_rows(),
            2,
            "width change rebuilds: 20 chars wrap to 2 rows at 10"
        );
        assert_eq!(
            cache.cleaned(40).total_rows(),
            1,
            "flipping back rebuilds again"
        );
    }

    #[test]
    fn cleaned_empty_cache_is_empty() {
        let mut cache = ViewportCache::build(&ChatView::default(), 10, 0, 0);
        let clean = cache.cleaned(10);
        assert_eq!(clean.total_rows(), 0);
        assert!(clean.texts().is_empty());
        assert_eq!(clean.visible_window(0, 10), (0, 0, 0));
    }
}
