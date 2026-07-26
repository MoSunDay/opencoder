//! Core state for the vim-mode editing engine.
//!
//! [`VimState`] is the single value the engine mutates. It carries the buffer
//! text, a char-index cursor, the active mode, and small pending prefix state
//! (count, operator, `g`-sequence). It also retains the `original` text so a
//! discard exit (`:q!` / Ctrl+C) can restore the pre-edit content.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VimMode {
    Normal,
    Insert,
    Command,
    Search,
}

#[derive(Clone, Debug)]
pub struct VimState {
    pub text: String,
    pub cursor: usize, // char index into `text`
    pub original: String,
    pub mode: VimMode,
    pub cmdline: String,                     // command-line buffer (after `:`)
    pub search_input: String,                // search buffer (after `/` or `?`)
    pub search_forward: bool,                // direction of the current search entry
    pub last_search: Option<(String, bool)>, // (query, forward) of last Enter-ed search
    pub count: Option<usize>,                // pending count prefix
    pub pending_op: Option<char>,            // pending operator: 'd' | 'c' | 'y'
    pub pending_g: bool,                     // waiting for the second 'g' of `gg`
    pub register: String,                    // last yanked/deleted text
    pub register_linewise: bool,             // whether `register` was a linewise op
    pub status: String,                      // transient status message (e.g. search wrap)
}

impl VimState {
    /// Start in Insert mode with the cursor at the end of the text (ready to
    /// append). `original` is retained to detect modification on exit.
    pub fn new(text: String) -> Self {
        let cursor = text.chars().count();
        let original = text.clone();
        Self {
            text,
            cursor,
            original,
            mode: VimMode::Insert,
            cmdline: String::new(),
            search_input: String::new(),
            search_forward: true,
            last_search: None,
            count: None,
            pending_op: None,
            pending_g: false,
            register: String::new(),
            register_linewise: false,
            status: String::new(),
        }
    }

    pub fn is_modified(&self) -> bool {
        self.text != self.original
    }

    /// Mark the current text as saved (used by `:w` so is_modified clears while
    /// editing continues).
    pub fn mark_saved(&mut self) {
        self.original = self.text.clone();
    }

    /// Label shown in the editor border. For Command/Search modes the in-progress
    /// input is included so the user sees what they are typing.
    pub fn mode_label(&self) -> String {
        match self.mode {
            VimMode::Normal => "NORMAL".to_string(),
            VimMode::Insert => "INSERT".to_string(),
            VimMode::Command => format!(":{}", self.cmdline),
            VimMode::Search => {
                let sep = if self.search_forward { '/' } else { '?' };
                format!("{}{}", sep, self.search_input)
            }
        }
    }

    /// Clear all pending prefix/operator state (called after any completed or
    /// aborted normal-mode command).
    pub fn reset_pending(&mut self) {
        self.count = None;
        self.pending_op = None;
        self.pending_g = false;
    }

    pub fn char_count(&self) -> usize {
        self.text.chars().count()
    }

    /// Clamp cursor into the valid range [0, char_count].
    pub fn clamp_cursor(&mut self) {
        let len = self.char_count();
        if self.cursor > len {
            self.cursor = len;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VimAction {
    Continue,
    Exit,
}

/// True when the key is a Ctrl+C chord (the canonical `\u{3}` ETX char or a
/// `Char('c')` paired with CONTROL). Module-public so both `insert` and
/// `normal` can reuse it.
pub(crate) fn is_ctrl_c(k: &KeyEvent) -> bool {
    matches!(k.code, KeyCode::Char('\u{3}'))
        || (matches!(k.code, KeyCode::Char('c')) && k.modifiers.contains(KeyModifiers::CONTROL))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_starts_in_insert_at_end() {
        let s = VimState::new("abc".to_string());
        assert_eq!(s.mode, VimMode::Insert);
        assert_eq!(s.cursor, 3);
        assert_eq!(s.original, "abc");
        assert!(!s.is_modified());
    }

    #[test]
    fn mode_label_reflects_mode_and_buffers() {
        let mut s = VimState::new("x".to_string());
        assert_eq!(s.mode_label(), "INSERT");
        s.mode = VimMode::Normal;
        assert_eq!(s.mode_label(), "NORMAL");
        s.mode = VimMode::Command;
        s.cmdline = "wq".to_string();
        assert_eq!(s.mode_label(), ":wq");
        s.mode = VimMode::Search;
        s.search_forward = true;
        s.search_input = "foo".to_string();
        assert_eq!(s.mode_label(), "/foo");
        s.search_forward = false;
        assert_eq!(s.mode_label(), "?foo");
    }

    #[test]
    fn is_modified_tracks_text_changes() {
        let mut s = VimState::new("abc".to_string());
        assert!(!s.is_modified());
        s.text.push_str("def");
        assert!(s.is_modified());
        s.mark_saved();
        assert!(!s.is_modified());
        s.text.clear();
        assert!(s.is_modified());
    }

    #[test]
    fn clamp_cursor_caps_to_char_count() {
        let mut s = VimState::new("abc".to_string()); // len 3
        s.cursor = 99;
        s.clamp_cursor();
        assert_eq!(s.cursor, 3);
        // cursor at end is allowed (Insert can append).
        s.cursor = 3;
        s.clamp_cursor();
        assert_eq!(s.cursor, 3);
        s.cursor = 0;
        s.clamp_cursor();
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn reset_pending_clears_all_prefix_state() {
        let mut s = VimState::new("abc".to_string());
        s.count = Some(5);
        s.pending_op = Some('d');
        s.pending_g = true;
        s.reset_pending();
        assert_eq!(s.count, None);
        assert_eq!(s.pending_op, None);
        assert!(!s.pending_g);
    }

    #[test]
    fn is_ctrl_c_detects_both_chords() {
        let etx = KeyEvent::new(KeyCode::Char('\u{3}'), KeyModifiers::NONE);
        assert!(is_ctrl_c(&etx));
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(is_ctrl_c(&ctrl_c));
        let plain_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE);
        assert!(!is_ctrl_c(&plain_c));
    }
}
