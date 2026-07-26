//! Insert-mode key handling for the vim engine.
//!
//! Plain `Enter` saves & exits; newlines are inserted via Shift+Enter /
//! Alt+Enter / Ctrl+J chords (mirroring the composer input box). `Esc` returns
//! to Normal mode (moving the cursor one char left, as vim does). Up/Down are
//! handled at the top level (they need `inner_w`/`prompt_w`) so this module
//! does not touch them.

use super::state::{is_ctrl_c, VimAction, VimMode};
use crate::composer;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub(crate) fn is_newline_chord(k: &KeyEvent) -> bool {
    (k.code == KeyCode::Enter
        && k.modifiers
            .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT))
        || (k.code == KeyCode::Char('j') && k.modifiers.contains(KeyModifiers::CONTROL))
}

pub fn handle_insert(state: &mut super::state::VimState, k: KeyEvent) -> VimAction {
    // Ctrl+C: discard edits (restore original) and exit.
    if is_ctrl_c(&k) {
        state.text = state.original.clone();
        state.clamp_cursor();
        return VimAction::Exit;
    }
    // Newline chords (Shift/Ctrl+Enter, Ctrl+J) insert a newline and stay in
    // Insert. Checked before the Enter/Char arms because the chord may arrive
    // as either KeyCode::Enter (Shift+Enter) or KeyCode::Char('j') (Ctrl+J).
    if is_newline_chord(&k) {
        let (t, i) = composer::insert_newline(&state.text, state.cursor);
        state.text = t;
        state.cursor = i;
        return VimAction::Continue;
    }
    match k.code {
        KeyCode::Esc => {
            state.mode = VimMode::Normal;
            // vim: leaving insert moves cursor one char left (if possible).
            if state.cursor > 0 {
                state.cursor -= 1;
            }
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Enter => {
            // Enter (plain) saves & exits. Newlines are inserted via chords.
            VimAction::Exit
        }
        KeyCode::Char(c) if !c.is_control() => {
            let (t, i) = composer::insert_char(&state.text, state.cursor, c);
            state.text = t;
            state.cursor = i;
            VimAction::Continue
        }
        KeyCode::Backspace => {
            if let Some((t, i)) = composer::backspace(&state.text, state.cursor) {
                state.text = t;
                state.cursor = i;
            }
            VimAction::Continue
        }
        KeyCode::Left => {
            state.cursor = state.cursor.saturating_sub(1);
            VimAction::Continue
        }
        KeyCode::Right => {
            let len = state.char_count();
            if state.cursor < len {
                state.cursor += 1;
            }
            VimAction::Continue
        }
        _ => VimAction::Continue,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composer;
    use crate::vim::state::VimState;

    fn k(c: char) -> KeyEvent {
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
    fn shift_enter() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)
    }

    #[test]
    fn inserts_chars_and_advances_cursor() {
        let mut s = VimState::new("ac".to_string());
        s.cursor = 1; // between a and c
        assert_eq!(handle_insert(&mut s, k('b')), VimAction::Continue);
        assert_eq!(s.text, "abc");
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn backspace_deletes_before_cursor() {
        let mut s = VimState::new("abc".to_string());
        s.cursor = 3;
        assert_eq!(handle_insert(&mut s, backspace()), VimAction::Continue);
        assert_eq!(s.text, "ab");
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn esc_returns_to_normal_and_moves_left() {
        let mut s = VimState::new("abc".to_string());
        s.cursor = 3;
        assert_eq!(handle_insert(&mut s, esc()), VimAction::Continue);
        assert_eq!(s.mode, VimMode::Normal);
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn esc_at_zero_keeps_zero() {
        let mut s = VimState::new("abc".to_string());
        s.cursor = 0;
        handle_insert(&mut s, esc());
        assert_eq!(s.mode, VimMode::Normal);
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn plain_enter_exits() {
        let mut s = VimState::new("abc".to_string());
        s.cursor = 3;
        assert_eq!(handle_insert(&mut s, enter()), VimAction::Exit);
    }

    #[test]
    fn newline_chord_inserts_newline() {
        let mut s = VimState::new("ab".to_string());
        s.cursor = 1;
        handle_insert(&mut s, shift_enter());
        assert_eq!(s.text, "a\nb");
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn ctrl_c_discards_and_exits() {
        let mut s = VimState::new("orig".to_string());
        s.text = "changed".to_string();
        s.cursor = 7;
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(handle_insert(&mut s, ctrl_c), VimAction::Exit);
        assert_eq!(s.text, "orig");
        assert!(!s.is_modified());
    }

    #[test]
    fn arrow_keys_move_within_bounds() {
        let mut s = VimState::new("abc".to_string());
        s.cursor = 3;
        handle_insert(&mut s, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(s.cursor, 2);
        handle_insert(&mut s, KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(s.cursor, 3);
        // right at end stays put
        handle_insert(&mut s, KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(s.cursor, 3);
    }

    #[test]
    fn newline_chord_detection() {
        assert!(is_newline_chord(&shift_enter()));
        assert!(is_newline_chord(&KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::ALT
        )));
        assert!(is_newline_chord(&KeyEvent::new(
            KeyCode::Char('j'),
            KeyModifiers::CONTROL
        )));
        assert!(!is_newline_chord(&enter()));
        assert!(!is_newline_chord(&k('j')));
        // sanity: composer::insert_newline shape
        let (t, i) = composer::insert_newline("ab", 1);
        assert_eq!(t, "a\nb");
        assert_eq!(i, 2);
    }
}
