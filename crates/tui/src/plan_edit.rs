//! Plan text editor — full-area editor for the plan card.
//!
//! Reuses the pure composer cursor/wrap functions. Enter (or Ctrl+C) saves the
//! edit and returns to the normal display; readline shortcuts Ctrl+A / Ctrl+E /
//! Ctrl+W mirror the main input box. Vim-style Normal mode (h/j/k/l, `i` to
//! insert) is retained for cursor navigation.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::composer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanEditMode {
    Normal,
    Insert,
}

/// Mutable state for the plan editor. `original` is retained so we can
/// detect whether the user changed anything before persisting.
#[derive(Clone, Debug)]
pub struct PlanEdit {
    pub text: String,
    pub cursor: usize,
    pub original: String,
    pub mode: PlanEditMode,
}

impl PlanEdit {
    pub fn new(text: String) -> Self {
        let cursor = text.chars().count();
        Self {
            cursor,
            original: text.clone(),
            text,
            mode: PlanEditMode::Insert,
        }
    }

    pub fn is_modified(&self) -> bool {
        self.text != self.original
    }

    pub fn mode_label(&self) -> &'static str {
        match self.mode {
            PlanEditMode::Normal => "NORMAL",
            PlanEditMode::Insert => "INSERT",
        }
    }
}

/// What the app loop should do after handling a key.
#[derive(Debug, PartialEq, Eq)]
pub enum PlanEditAction {
    Continue,
    /// Save (if modified) and leave the plan editor.
    Exit,
}

/// Handle a key in plan-edit mode.
pub fn handle_plan_edit_key(
    pe: &mut PlanEdit,
    k: KeyEvent,
    inner_w: u16,
    prompt_w: u16,
) -> PlanEditAction {
    // Ctrl+C works in both modes (crossterm may deliver it as Char('c')+CONTROL
    // or as the raw ETX control char 0x03).
    if is_ctrl_c(&k) {
        return PlanEditAction::Exit;
    }
    // Enter saves & exits the plan editor (both modes).
    if k.code == KeyCode::Enter {
        return PlanEditAction::Exit;
    }
    // Readline-style shortcuts shared with the main input box (both modes).
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        match k.code {
            // Ctrl+A / Ctrl+E: cursor to start / end.
            KeyCode::Char('a') => {
                pe.cursor = 0;
                return PlanEditAction::Continue;
            }
            KeyCode::Char('e') => {
                pe.cursor = pe.text.chars().count();
                return PlanEditAction::Continue;
            }
            // Ctrl+W: delete the word before the cursor.
            KeyCode::Char('w') => {
                if let Some((t, i)) = composer::delete_word_back(&pe.text, pe.cursor) {
                    pe.text = t;
                    pe.cursor = i;
                }
                return PlanEditAction::Continue;
            }
            _ => {}
        }
    }
    match pe.mode {
        PlanEditMode::Insert => handle_insert(pe, k, inner_w, prompt_w),
        PlanEditMode::Normal => handle_normal(pe, k, inner_w, prompt_w),
    }
}

fn is_ctrl_c(k: &KeyEvent) -> bool {
    matches!(k.code, KeyCode::Char('\u{3}'))
        || (matches!(k.code, KeyCode::Char('c')) && k.modifiers.contains(KeyModifiers::CONTROL))
}

fn handle_insert(pe: &mut PlanEdit, k: KeyEvent, inner_w: u16, prompt_w: u16) -> PlanEditAction {
    match k.code {
        KeyCode::Esc => {
            pe.mode = PlanEditMode::Normal;
            PlanEditAction::Continue
        }
        KeyCode::Char(c) if !c.is_control() => {
            let (t, idx) = composer::insert_char(&pe.text, pe.cursor, c);
            pe.text = t;
            pe.cursor = idx;
            PlanEditAction::Continue
        }
        KeyCode::Backspace => {
            if let Some((t, idx)) = composer::backspace(&pe.text, pe.cursor) {
                pe.text = t;
                pe.cursor = idx;
            }
            PlanEditAction::Continue
        }
        KeyCode::Left => {
            pe.cursor = pe.cursor.saturating_sub(1);
            PlanEditAction::Continue
        }
        KeyCode::Right => {
            let len = pe.text.chars().count();
            if pe.cursor < len {
                pe.cursor += 1;
            }
            PlanEditAction::Continue
        }
        KeyCode::Up => {
            pe.cursor = composer::move_cursor_vertical(&pe.text, pe.cursor, -1, inner_w, prompt_w);
            PlanEditAction::Continue
        }
        KeyCode::Down => {
            pe.cursor = composer::move_cursor_vertical(&pe.text, pe.cursor, 1, inner_w, prompt_w);
            PlanEditAction::Continue
        }
        _ => PlanEditAction::Continue,
    }
}

fn handle_normal(pe: &mut PlanEdit, k: KeyEvent, inner_w: u16, prompt_w: u16) -> PlanEditAction {
    match k.code {
        KeyCode::Esc => PlanEditAction::Exit,
        KeyCode::Char('i') | KeyCode::Char('I') => {
            pe.mode = PlanEditMode::Insert;
            PlanEditAction::Continue
        }
        KeyCode::Char('h') => {
            pe.cursor = pe.cursor.saturating_sub(1);
            PlanEditAction::Continue
        }
        KeyCode::Char('l') => {
            let len = pe.text.chars().count();
            if pe.cursor < len {
                pe.cursor += 1;
            }
            PlanEditAction::Continue
        }
        KeyCode::Char('j') => {
            pe.cursor = composer::move_cursor_vertical(&pe.text, pe.cursor, 1, inner_w, prompt_w);
            PlanEditAction::Continue
        }
        KeyCode::Char('k') => {
            pe.cursor = composer::move_cursor_vertical(&pe.text, pe.cursor, -1, inner_w, prompt_w);
            PlanEditAction::Continue
        }
        _ => PlanEditAction::Continue,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pe(text: &str) -> PlanEdit {
        PlanEdit::new(text.to_string())
    }

    #[test]
    fn new_starts_in_insert_mode_at_end() {
        let e = pe("hello");
        assert_eq!(e.mode, PlanEditMode::Insert);
        assert_eq!(e.cursor, 5);
        assert!(!e.is_modified());
    }

    #[test]
    fn esc_switches_to_normal() {
        let mut e = pe("hello");
        let action = handle_plan_edit_key(&mut e, key_esc(), 80, 2);
        assert_eq!(action, PlanEditAction::Continue);
        assert_eq!(e.mode, PlanEditMode::Normal);
    }

    #[test]
    fn esc_in_normal_exits() {
        let mut e = pe("hello");
        // Enter normal first
        handle_plan_edit_key(&mut e, key_esc(), 80, 2);
        assert_eq!(e.mode, PlanEditMode::Normal);
        // Esc again exits
        let action = handle_plan_edit_key(&mut e, key_esc(), 80, 2);
        assert_eq!(action, PlanEditAction::Exit);
    }

    #[test]
    fn ctrl_c_exits_from_insert() {
        let mut e = pe("hello");
        let action = handle_plan_edit_key(&mut e, key_ctrl_c(), 80, 2);
        assert_eq!(action, PlanEditAction::Exit);
    }

    #[test]
    fn ctrl_c_exits_from_normal() {
        let mut e = pe("hello");
        handle_plan_edit_key(&mut e, key_esc(), 80, 2); // -> Normal
        let action = handle_plan_edit_key(&mut e, key_ctrl_c(), 80, 2);
        assert_eq!(action, PlanEditAction::Exit);
    }

    #[test]
    fn insert_char_appends() {
        let mut e = pe("ab");
        // cursor at end (2)
        assert_eq!(e.cursor, 2);
        handle_plan_edit_key(&mut e, key_char('x'), 80, 2);
        assert_eq!(e.text, "abx");
        assert_eq!(e.cursor, 3);
        assert!(e.is_modified());
    }

    #[test]
    fn backspace_removes_char() {
        let mut e = pe("abc");
        e.cursor = 3;
        handle_plan_edit_key(&mut e, key(KeyCode::Backspace), 80, 2);
        assert_eq!(e.text, "ab");
        assert_eq!(e.cursor, 2);
    }

    #[test]
    fn enter_saves_and_exits() {
        let mut e = pe("ab");
        e.cursor = 1;
        let action = handle_plan_edit_key(&mut e, key(KeyCode::Enter), 80, 2);
        assert_eq!(action, PlanEditAction::Exit);
        assert_eq!(e.text, "ab");
    }

    #[test]
    fn ctrl_a_moves_cursor_to_start() {
        let mut e = pe("hello");
        e.cursor = 3;
        let action = handle_plan_edit_key(&mut e, ctrl('a'), 80, 2);
        assert_eq!(action, PlanEditAction::Continue);
        assert_eq!(e.cursor, 0);
    }

    #[test]
    fn ctrl_e_moves_cursor_to_end() {
        let mut e = pe("hello");
        e.cursor = 1;
        let action = handle_plan_edit_key(&mut e, ctrl('e'), 80, 2);
        assert_eq!(action, PlanEditAction::Continue);
        assert_eq!(e.cursor, 5);
    }

    #[test]
    fn ctrl_w_deletes_word_back() {
        let mut e = pe("hello world");
        e.cursor = 11;
        let action = handle_plan_edit_key(&mut e, ctrl('w'), 80, 2);
        assert_eq!(action, PlanEditAction::Continue);
        assert_eq!(e.text, "hello ");
        assert_eq!(e.cursor, 6);
    }

    #[test]
    fn normal_h_l_move_cursor() {
        let mut e = pe("hello");
        e.cursor = 3;
        e.mode = PlanEditMode::Normal;
        handle_plan_edit_key(&mut e, key_char('h'), 80, 2);
        assert_eq!(e.cursor, 2);
        handle_plan_edit_key(&mut e, key_char('l'), 80, 2);
        assert_eq!(e.cursor, 3);
    }

    #[test]
    fn normal_i_enters_insert() {
        let mut e = pe("hello");
        e.mode = PlanEditMode::Normal;
        handle_plan_edit_key(&mut e, key_char('i'), 80, 2);
        assert_eq!(e.mode, PlanEditMode::Insert);
    }

    #[test]
    fn left_right_in_insert() {
        let mut e = pe("abc");
        // cursor at end
        handle_plan_edit_key(&mut e, key(KeyCode::Left), 80, 2);
        assert_eq!(e.cursor, 2);
        handle_plan_edit_key(&mut e, key(KeyCode::Left), 80, 2);
        assert_eq!(e.cursor, 1);
        handle_plan_edit_key(&mut e, key(KeyCode::Right), 80, 2);
        assert_eq!(e.cursor, 2);
    }

    #[test]
    fn control_char_not_inserted() {
        let mut e = pe("ab");
        // U+0001 (SOH) is a control char — should be ignored
        handle_plan_edit_key(&mut e, key_char('\u{1}'), 80, 2);
        assert_eq!(e.text, "ab");
    }

    // --- helpers ---

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }

    fn key_char(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), crossterm::event::KeyModifiers::NONE)
    }

    fn key_esc() -> KeyEvent {
        key(KeyCode::Esc)
    }

    fn key_ctrl_c() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }
}
