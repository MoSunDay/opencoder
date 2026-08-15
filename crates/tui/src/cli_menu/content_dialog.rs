//! Multi-line editor dialog for the `/cli` form's `content` field.
//!
//! Tool usage contracts can span many lines, which the single-line form field
//! cannot hold. Focusing `content` and pressing Enter opens this overlay;
//! typing/Enter/Backspace edit verbatim, Ctrl+S applies back to the form,
//! Esc discards. Inside the dialog Enter inserts a newline — it never saves.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Visible logical lines in the dialog viewport (excl. borders/title).
pub const VIEW_LINES: usize = 8;

#[derive(Debug, PartialEq, Eq)]
pub enum ContentOutcome {
    Idle,
    /// Write `text` back to the form's content field and close.
    Apply,
    /// Discard edits and close.
    Cancel,
}

#[derive(Debug, Default)]
pub struct ContentDialog {
    pub text: String,
    /// Cursor as a char index into `text`.
    pub cursor: usize,
    /// Index of the first visible logical line (vertical scroll).
    pub scroll: usize,
}

impl ContentDialog {
    pub fn new(text: String, cursor: usize) -> Self {
        let mut d = Self {
            cursor: cursor.min(text.chars().count()),
            text,
            scroll: 0,
        };
        d.clamp_scroll();
        d
    }

    /// Insert text verbatim (newlines included) at the cursor — the paste path.
    pub fn insert_text(&mut self, s: &str) {
        insert_at(&mut self.text, &mut self.cursor, s);
        self.clamp_scroll();
    }

    /// (line, column) of the cursor, both in chars.
    pub fn line_col(&self) -> (usize, usize) {
        line_col(&self.text, self.cursor)
    }

    fn clamp_scroll(&mut self) {
        let (line, _) = self.line_col();
        if line < self.scroll {
            self.scroll = line;
        }
        let last_visible = self.scroll + VIEW_LINES;
        if line >= last_visible {
            self.scroll = line + 1 - VIEW_LINES;
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> ContentOutcome {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return match key.code {
                KeyCode::Char('s') | KeyCode::Char('\u{13}') => ContentOutcome::Apply,
                KeyCode::Char('u') | KeyCode::Char('\u{15}') => {
                    self.text.clear();
                    self.cursor = 0;
                    self.scroll = 0;
                    ContentOutcome::Idle
                }
                _ => ContentOutcome::Idle,
            };
        }
        match key.code {
            KeyCode::Esc => ContentOutcome::Cancel,
            KeyCode::Enter => {
                self.insert_text("\n");
                ContentOutcome::Idle
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    let byte = char_to_byte(&self.text, self.cursor);
                    self.text.remove(byte);
                    self.clamp_scroll();
                }
                ContentOutcome::Idle
            }
            KeyCode::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                ContentOutcome::Idle
            }
            KeyCode::Right => {
                self.cursor = (self.cursor + 1).min(self.text.chars().count());
                ContentOutcome::Idle
            }
            KeyCode::Up => {
                self.move_vertical(-1);
                ContentOutcome::Idle
            }
            KeyCode::Down => {
                self.move_vertical(1);
                ContentOutcome::Idle
            }
            KeyCode::Char(c) => {
                self.insert_text(&c.to_string());
                ContentOutcome::Idle
            }
            _ => ContentOutcome::Idle,
        }
    }

    /// Move the cursor one logical line up/down, keeping the column.
    fn move_vertical(&mut self, delta: i32) {
        let (line, col) = self.line_col();
        let target = line as i32 + delta;
        if target < 0 {
            return;
        }
        let total_lines = self.text.split('\n').count() as i32;
        if target >= total_lines {
            // Past the last line: park the cursor at the end of the text.
            self.cursor = self.text.chars().count();
        } else {
            self.cursor = offset_for_line_col(&self.text, target as usize, col);
        }
        self.clamp_scroll();
    }
}

fn char_to_byte(text: &str, cursor: usize) -> usize {
    text.char_indices()
        .nth(cursor)
        .map(|(i, _)| i)
        .unwrap_or(text.len())
}

fn insert_at(text: &mut String, cursor: &mut usize, s: &str) {
    let byte = char_to_byte(text, *cursor);
    text.insert_str(byte, s);
    *cursor += s.chars().count();
}

fn line_col(text: &str, cursor: usize) -> (usize, usize) {
    let mut line = 0;
    let mut col = 0;
    for (i, c) in text.chars().enumerate() {
        if i >= cursor {
            break;
        }
        if c == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

fn offset_for_line_col(text: &str, line: usize, col: usize) -> usize {
    let mut cur_line = 0;
    let mut cur_col = 0;
    let mut offset = 0;
    for c in text.chars() {
        if cur_line == line && cur_col >= col {
            break;
        }
        if c == '\n' {
            if cur_line == line {
                break;
            }
            cur_line += 1;
            cur_col = 0;
        } else {
            cur_col += 1;
        }
        offset += 1;
    }
    offset.min(text.chars().count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn typing_and_enter_build_multiline_text() {
        let mut d = ContentDialog::default();
        for k in "ab".chars() {
            d.handle_key(key(KeyCode::Char(k)));
        }
        d.handle_key(key(KeyCode::Enter));
        for k in "cd".chars() {
            d.handle_key(key(KeyCode::Char(k)));
        }
        assert_eq!(d.text, "ab\ncd");
        assert_eq!(d.line_col(), (1, 2));
    }

    #[test]
    fn backspace_removes_char_before_cursor() {
        let mut d = ContentDialog::new("abc".into(), 2);
        d.handle_key(key(KeyCode::Backspace));
        assert_eq!(d.text, "ac");
        assert_eq!(d.cursor, 1);
        // backspace at position 0 is a no-op
        let mut d = ContentDialog::new("x".into(), 0);
        d.handle_key(key(KeyCode::Backspace));
        assert_eq!(d.text, "x");
    }

    #[test]
    fn vertical_moves_keep_column() {
        // line 2 "longer" has 6 chars: cursor 16 is its end.
        let mut d = ContentDialog::new("abcdef\nxy\nlonger".into(), 16);
        d.handle_key(key(KeyCode::Up)); // to line 1, col 2
        assert_eq!(d.line_col(), (1, 2));
        d.handle_key(key(KeyCode::Up)); // to line 0, col 2
        assert_eq!(d.line_col(), (0, 2));
        d.handle_key(key(KeyCode::Down));
        assert_eq!(d.line_col(), (1, 2));
    }

    #[test]
    fn down_past_last_line_parks_at_end() {
        let mut d = ContentDialog::new("ab\ncd".into(), 5);
        d.handle_key(key(KeyCode::Down));
        assert_eq!(d.cursor, d.text.chars().count());
    }

    #[test]
    fn ctrl_s_applies_ctrl_u_clears_esc_cancels() {
        let mut d = ContentDialog::new("keep".into(), 4);
        d.handle_key(key(KeyCode::Char('x')));
        assert_eq!(d.handle_key(ctrl('s')), ContentOutcome::Apply);
        assert_eq!(d.text, "keepx", "apply does not mutate the buffer");

        let mut d = ContentDialog::new("keep".into(), 4);
        assert_eq!(d.handle_key(ctrl('u')), ContentOutcome::Idle);
        assert!(d.text.is_empty());
        assert_eq!(d.cursor, 0);

        let mut d = ContentDialog::new("keep".into(), 4);
        assert_eq!(d.handle_key(key(KeyCode::Esc)), ContentOutcome::Cancel);
        assert_eq!(d.text, "keep", "cancel does not mutate the buffer");
    }

    #[test]
    fn insert_text_pastes_verbatim_with_newlines() {
        let mut d = ContentDialog::new("one".into(), 3);
        d.insert_text("two\nthree\n");
        assert_eq!(d.text, "onetwo\nthree\n");
        assert_eq!(d.line_col(), (2, 0), "cursor lands after the pasted text");
    }

    #[test]
    fn scroll_follows_cursor_line() {
        let text = (0..=10).map(|i| i.to_string()).collect::<Vec<_>>().join("\n");
        let total = text.chars().count();
        let mut d = ContentDialog::new(text, total);
        // 11 lines with an 8-line viewport: end-of-text cursor on line 10
        // already forces scroll to 3 on construction.
        assert_eq!(d.scroll, 3);
        // walk the cursor up until it leaves the viewport; scroll must follow
        for _ in 0..25 {
            d.handle_key(key(KeyCode::Up));
        }
        assert!(
            d.scroll + VIEW_LINES > d.line_col().0,
            "cursor line must stay visible"
        );
        assert!(d.line_col().0 >= d.scroll);
        // and back down to the end
        for _ in 0..40 {
            d.handle_key(key(KeyCode::Down));
        }
        assert!(d.scroll + VIEW_LINES > d.line_col().0);
    }

    #[test]
    fn multibyte_chars_count_as_one() {
        let mut d = ContentDialog::new("中文".into(), 0);
        d.handle_key(key(KeyCode::Right));
        d.handle_key(key(KeyCode::Char('！')));
        assert_eq!(d.text, "中！文");
        assert_eq!(d.cursor, 2);
    }
}
