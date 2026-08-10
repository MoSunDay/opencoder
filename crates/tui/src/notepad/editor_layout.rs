//! Pure visual-row layout for the notepad editor.
//!
//! Rendering, cursor placement, vertical motion, and scrolling all derive
//! from this module so a soft-wrapped line cannot disagree across paths.

use crate::composer::{self, VisualRow};

/// One screen row produced from the editor buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditorRow {
    pub range: VisualRow,
    /// Present only on the first visual row of a logical line.
    pub line_number: Option<usize>,
}

/// Immutable visual layout for a buffer at one editor text width.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorLayout {
    rows: Vec<EditorRow>,
    chars: Vec<char>,
}

impl EditorLayout {
    pub fn new(text: &str, width: u16) -> Self {
        let chars: Vec<char> = text.chars().collect();
        let wrapped = composer::wrap_rows(text, width, 0);
        let mut rows = Vec::with_capacity(wrapped.len());
        let mut scan = 0usize;
        let mut line_number = 1usize;

        for range in wrapped {
            while scan < range.start {
                if chars[scan] == '\n' {
                    line_number += 1;
                }
                scan += 1;
            }
            let starts_logical_line = range.start == 0 || chars[range.start - 1] == '\n';
            rows.push(EditorRow {
                range,
                line_number: starts_logical_line.then_some(line_number),
            });
        }

        Self { rows, chars }
    }

    pub fn rows(&self) -> &[EditorRow] {
        &self.rows
    }

    pub fn row_text(&self, row: EditorRow) -> String {
        self.chars[row.range.start..row.range.end].iter().collect()
    }

    pub fn cursor_position(&self, cursor: usize) -> (usize, usize) {
        let cursor = cursor.min(self.chars.len());
        let row = self.cursor_row(cursor);
        let start = self.rows[row].range.start;
        let col = self.chars[start..cursor]
            .iter()
            .copied()
            .map(composer::char_width)
            .sum();
        (row, col)
    }

    pub fn move_cursor_rows(&self, cursor: usize, delta: isize) -> usize {
        let (row, col) = self.cursor_position(cursor);
        let target = row
            .saturating_add_signed(delta)
            .min(self.rows.len().saturating_sub(1));
        self.index_at_column(target, col)
    }

    pub fn clamp_scroll(&self, scroll: usize, height: usize) -> usize {
        scroll.min(self.rows.len().saturating_sub(height))
    }

    fn cursor_row(&self, cursor: usize) -> usize {
        self.rows
            .iter()
            // Soft-wrapped rows share a boundary (`previous.end == next.start`).
            // Vim's Normal cursor at that char belongs to the next row, unlike
            // the composer's insertion caret, so prefer the last matching row.
            .rposition(|row| row.range.start <= cursor && cursor <= row.range.end)
            .unwrap_or_else(|| self.rows.len().saturating_sub(1))
    }

    fn index_at_column(&self, row: usize, target_col: usize) -> usize {
        let range = self.rows[row].range;
        let mut col = 0usize;
        let mut index = range.start;
        for (offset, ch) in self.chars[range.start..range.end]
            .iter()
            .copied()
            .enumerate()
        {
            let next = col + composer::char_width(ch);
            if next > target_col {
                break;
            }
            col = next;
            index = range.start + offset + 1;
        }
        index
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_line_wraps_without_losing_text() {
        let layout = EditorLayout::new("alpha beta gamma", 6);
        let rendered: String = layout
            .rows()
            .iter()
            .map(|row| layout.row_text(*row))
            .collect();
        assert_eq!(rendered, "alpha beta gamma");
        assert!(layout.rows().len() > 1);
        assert_eq!(layout.rows()[0].line_number, Some(1));
        assert!(layout.rows()[1..]
            .iter()
            .all(|row| row.line_number.is_none()));
    }

    #[test]
    fn explicit_and_trailing_newlines_get_line_numbers() {
        let layout = EditorLayout::new("abcdef\n\n尾行\n", 3);
        let numbered: Vec<usize> = layout
            .rows()
            .iter()
            .filter_map(|row| row.line_number)
            .collect();
        assert_eq!(numbered, vec![1, 2, 3, 4]);
    }

    #[test]
    fn cjk_wrap_and_cursor_use_display_width() {
        let layout = EditorLayout::new("你好世界", 4);
        assert_eq!(layout.rows().len(), 2);
        assert_eq!(layout.cursor_position(3), (1, 2));
        assert_eq!(layout.move_cursor_rows(1, 1), 3);
    }

    #[test]
    fn vertical_motion_does_not_stick_on_soft_wrap_boundary() {
        let layout = EditorLayout::new("abcdefghij", 4);
        let second_row = layout.move_cursor_rows(0, 1);
        assert_eq!(second_row, 4, "j must land on the next row's first char");
        assert_eq!(layout.cursor_position(second_row), (1, 0));
        let third_row = layout.move_cursor_rows(second_row, 1);
        assert_eq!(third_row, 8);
        assert_eq!(layout.cursor_position(third_row), (2, 0));
        assert_eq!(layout.move_cursor_rows(third_row, -1), second_row);
        assert_eq!(layout.move_cursor_rows(second_row, -1), 0);
    }

    #[test]
    fn explicit_newline_boundary_stays_on_previous_logical_line() {
        let layout = EditorLayout::new("abcd\nefgh", 4);
        assert_eq!(layout.cursor_position(4), (0, 4));
        assert_eq!(layout.cursor_position(5), (1, 0));
    }

    #[test]
    fn scrolling_clamps_to_last_full_page() {
        let layout = EditorLayout::new("a\nb\nc\nd", 10);
        assert_eq!(layout.clamp_scroll(99, 2), 2);
        assert_eq!(layout.clamp_scroll(99, 10), 0);
    }
}
