//! Plan text editor — a vim-mode editor wrapping [`crate::vim`].
//!
//! Opened with Shift+I in plan mode (idle). Starts in Insert mode (cursor at
//! the end) so the user can type immediately; pressing Esc drops to Normal mode
//! for full vim navigation, operators, search, and command-line.
//!
//! Exits:
//! - `Enter` (Normal/Insert) / `:wq` / `:x` — save & leave (text kept).
//! - `:q!` / `:q` / Ctrl+C — discard & leave (engine restores the original).
//!
//! The caller persists on [`PlanEditAction::Exit`] iff [`PlanEdit::is_modified`]
//! is true; discard exits already restore the original so this is automatic.

use crossterm::event::KeyEvent;

use crate::vim::{self, VimState};

/// Plan editor — a thin adapter over [`VimState`] exposing the contract the app
/// loop and renderer expect (text, cursor, mode label, modified flag).
#[derive(Clone, Debug)]
pub struct PlanEdit {
    vim: VimState,
}

impl PlanEdit {
    /// Seed from existing plan text. Starts in Insert mode, cursor at the end.
    pub fn new(text: String) -> Self {
        Self {
            vim: VimState::new(text),
        }
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
    fn new_starts_insert_cursor_at_end_unmodified() {
        let pe = PlanEdit::new("hello".to_string());
        assert_eq!(pe.text(), "hello");
        assert_eq!(pe.cursor(), 5);
        assert_eq!(pe.mode_label(), "INSERT");
        assert!(!pe.is_modified());
    }

    #[test]
    fn insert_appends_and_marks_modified() {
        let mut pe = PlanEdit::new("hi".to_string());
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
        // cursor at end (3). move left twice then backspace.
        handle_plan_edit_key(&mut pe, esc(), W, 2); // -> Normal
        handle_plan_edit_key(&mut pe, key('i'), W, 2); // back to Insert, cursor left
        handle_plan_edit_key(&mut pe, backspace(), W, 2);
        assert_eq!(pe.text(), "ac");
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
    fn enter_saves_and_exits() {
        let mut pe = PlanEdit::new("plan".to_string());
        handle_plan_edit_key(&mut pe, key('!'), W, 2);
        assert_eq!(
            handle_plan_edit_key(&mut pe, enter(), W, 2),
            PlanEditAction::Exit
        );
        // text retained -> modified
        assert_eq!(pe.text(), "plan!");
        assert!(pe.is_modified());
    }

    #[test]
    fn wq_saves_and_exits() {
        let mut pe = PlanEdit::new("x".to_string());
        handle_plan_edit_key(&mut pe, key('a'), W, 2);
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
        handle_plan_edit_key(&mut pe, key('Z'), W, 2);
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
    fn ctrl_c_discards_and_exits() {
        let mut pe = PlanEdit::new("base".to_string());
        handle_plan_edit_key(&mut pe, key('!'), W, 2);
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(
            handle_plan_edit_key(&mut pe, ctrl_c, W, 2),
            PlanEditAction::Exit
        );
        assert_eq!(pe.text(), "base");
        assert!(!pe.is_modified());
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
        // cursor at end (last char of line2). Move to Normal then dd.
        handle_plan_edit_key(&mut pe, esc(), W, 2);
        handle_plan_edit_key(&mut pe, key('d'), W, 2);
        handle_plan_edit_key(&mut pe, key('d'), W, 2);
        // only line1 remains
        assert_eq!(pe.text(), "line1");
    }
}
