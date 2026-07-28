//! Plan text editor — a vim-mode editor wrapping [`crate::vim`].
//!
//! Opened with Shift+I in plan mode (idle). Starts in Normal (view) mode,
//! cursor at the top, so the user can review the plan first; pressing `i`/`a`
//! enters Insert mode for editing, and Esc returns to Normal for full vim
//! navigation, operators, search, and command-line.
//!
//! Exits:
//! - `:wq` — save & leave (text kept).
//! - `:q!` / `:q` — discard & leave (engine restores the original).
//!
//! `Enter`, `Ctrl+C`, and `:x`/`:w` are no longer exit paths: in Insert mode
//! `Enter` inserts a newline and `Ctrl+C` returns to Normal; in Normal mode
//! both are no-ops.
//!
//! The caller persists on [`PlanEditAction::Exit`] iff [`PlanEdit::is_modified`]
//! is true; discard exits already restore the original so this is automatic.

use crossterm::event::KeyEvent;

use crate::vim::{self, VimMode, VimState};

/// Plan editor — a thin adapter over [`VimState`] exposing the contract the app
/// loop and renderer expect (text, cursor, mode label, modified flag).
#[derive(Clone, Debug)]
pub struct PlanEdit {
    vim: VimState,
}

impl PlanEdit {
    /// Seed from existing plan text. Starts in Normal (view) mode, cursor at
    /// the top, so the user can read the plan before pressing `i`/`a` to edit.
    pub fn new(text: String) -> Self {
        let mut vim = VimState::new(text);
        vim.mode = VimMode::Normal;
        vim.cursor = 0;
        Self { vim }
    }

    /// The current editor text.
    pub fn text(&self) -> &str {
        &self.vim.text
    }

    /// The current cursor position (char index).
    pub fn cursor(&self) -> usize {
        self.vim.cursor
    }

    /// Whether the buffer differs from the seed. False after a discard exit
    /// (the engine has restored the original).
    pub fn is_modified(&self) -> bool {
        self.vim.is_modified()
    }

    /// Label for the editor border. Includes the in-progress command/search
    /// input when in those modes (e.g. `:wq` or `/foo`).
    pub fn mode_label(&self) -> String {
        self.vim.mode_label()
    }
}

/// What the app loop should do after handling a plan-edit key.
#[derive(Debug, PartialEq, Eq)]
pub enum PlanEditAction {
    Continue,
    /// Leave the editor. Persist iff `PlanEdit::is_modified()`.
    Exit,
}

/// Handle a key in plan-edit mode by delegating to the vim engine.
pub fn handle_plan_edit_key(
    pe: &mut PlanEdit,
    k: KeyEvent,
    inner_w: u16,
    prompt_w: u16,
) -> PlanEditAction {
    match vim::handle_vim_key(&mut pe.vim, k, inner_w, prompt_w) {
        vim::VimAction::Continue => PlanEditAction::Continue,
        vim::VimAction::Exit => PlanEditAction::Exit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
    fn esc() -> KeyEvent {
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
    }
    fn enter() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
    }
    fn backspace() -> KeyEvent {
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)
    }
    const W: u16 = 80;

    #[test]
    fn new_starts_normal_cursor_at_top_unmodified() {
        let pe = PlanEdit::new("hello".to_string());
        assert_eq!(pe.text(), "hello");
        assert_eq!(pe.cursor(), 0);
        assert_eq!(pe.mode_label(), "NORMAL");
        assert!(!pe.is_modified());
    }

    #[test]
    fn insert_appends_and_marks_modified() {
        let mut pe = PlanEdit::new("hi".to_string());
        handle_plan_edit_key(&mut pe, key('A'), W, 2); // Normal -> Insert at end
        assert_eq!(
            handle_plan_edit_key(&mut pe, key('!'), W, 2),
            PlanEditAction::Continue
        );
        assert_eq!(pe.text(), "hi!");
        assert_eq!(pe.cursor(), 3);
        assert!(pe.is_modified());
    }

    #[test]
    fn backspace_in_insert_deletes() {
        let mut pe = PlanEdit::new("abc".to_string());
        // start in Normal (cursor 0); append at end then backspace.
        handle_plan_edit_key(&mut pe, key('A'), W, 2); // -> Insert at end (cursor 3)
        handle_plan_edit_key(&mut pe, backspace(), W, 2); // delete 'c'
        assert_eq!(pe.text(), "ab");
        assert_eq!(pe.cursor(), 2);
    }

    #[test]
    fn esc_drops_to_normal_then_i_returns_to_insert() {
        let mut pe = PlanEdit::new("abc".to_string());
        handle_plan_edit_key(&mut pe, esc(), W, 2);
        assert_eq!(pe.mode_label(), "NORMAL");
        handle_plan_edit_key(&mut pe, key('i'), W, 2);
        assert_eq!(pe.mode_label(), "INSERT");
    }

    #[test]
    fn wq_saves_and_exits() {
        let mut pe = PlanEdit::new("x".to_string());
        handle_plan_edit_key(&mut pe, key('A'), W, 2); // Normal -> Insert at end
        handle_plan_edit_key(&mut pe, key('a'), W, 2); // type 'a' -> "xa"
        handle_plan_edit_key(&mut pe, esc(), W, 2); // Normal
                                                    // type :wq
        handle_plan_edit_key(&mut pe, key(':'), W, 2);
        handle_plan_edit_key(&mut pe, key('w'), W, 2);
        handle_plan_edit_key(&mut pe, key('q'), W, 2);
        assert_eq!(
            handle_plan_edit_key(&mut pe, enter(), W, 2),
            PlanEditAction::Exit
        );
        assert_eq!(pe.text(), "xa");
        assert!(pe.is_modified());
    }

    #[test]
    fn q_bang_discards_and_exits_unmodified() {
        let mut pe = PlanEdit::new("orig".to_string());
        handle_plan_edit_key(&mut pe, key('A'), W, 2); // Normal -> Insert at end
        handle_plan_edit_key(&mut pe, key('Z'), W, 2); // type 'Z' -> "origZ"
        assert!(pe.is_modified());
        handle_plan_edit_key(&mut pe, esc(), W, 2); // Normal
        handle_plan_edit_key(&mut pe, key(':'), W, 2);
        handle_plan_edit_key(&mut pe, key('q'), W, 2);
        handle_plan_edit_key(&mut pe, key('!'), W, 2);
        assert_eq!(
            handle_plan_edit_key(&mut pe, enter(), W, 2),
            PlanEditAction::Exit
        );
        // discarded -> restored to original
        assert_eq!(pe.text(), "orig");
        assert!(!pe.is_modified());
    }

    #[test]
    fn ctrl_c_drops_to_normal() {
        let mut pe = PlanEdit::new("base".to_string());
        handle_plan_edit_key(&mut pe, key('A'), W, 2); // Normal -> Insert at end
        handle_plan_edit_key(&mut pe, key('!'), W, 2); // text="base!"
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(
            handle_plan_edit_key(&mut pe, ctrl_c, W, 2),
            PlanEditAction::Continue
        );
        assert_eq!(pe.mode_label(), "NORMAL");
        // text retained, still modified
        assert_eq!(pe.text(), "base!");
        assert!(pe.is_modified());
    }

    #[test]
    fn search_navigates_cursor() {
        let mut pe = PlanEdit::new("foo bar baz".to_string());
        handle_plan_edit_key(&mut pe, esc(), W, 2); // Normal (cursor at 'z' area after left)
                                                    // search forward for "bar"
        handle_plan_edit_key(&mut pe, key('/'), W, 2);
        for c in "bar".chars() {
            handle_plan_edit_key(&mut pe, key(c), W, 2);
        }
        handle_plan_edit_key(&mut pe, enter(), W, 2);
        assert_eq!(pe.mode_label(), "NORMAL");
        // cursor should be at the 'b' of "bar" (char index 4)
        assert_eq!(pe.cursor(), 4);
    }

    #[test]
    fn dd_deletes_current_line() {
        let mut pe = PlanEdit::new("line1\nline2".to_string());
        // start in Normal at cursor 0 (line1); jump to last line then dd.
        handle_plan_edit_key(&mut pe, key('G'), W, 2); // cursor to last line
        handle_plan_edit_key(&mut pe, key('d'), W, 2);
        handle_plan_edit_key(&mut pe, key('d'), W, 2);
        // only line1 remains
        assert_eq!(pe.text(), "line1");
    }
}
