//! Insert-mode key handling for the vim engine.
//!
//! `Enter` inserts a newline (plain Enter) so multi-line input is natural,
//! matching vim; `Esc` and `Ctrl+C` both return to Normal mode (moving the
//! cursor one char left, as vim does) without exiting. There is no direct
//! exit from Insert mode — use `:wq`/`:q` in Command mode to leave the editor.
//! Up/Down are handled at the top level (they need `inner_w`/`prompt_w`) so
//! this module does not touch them.

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
    // Ctrl+C: drop to Normal mode (same as Esc); no exit, no discard.
    if is_ctrl_c(&k) {
        state.mode = VimMode::Normal;
        if state.cursor > 0 {
            state.cursor -= 1;
        }
        state.reset_pending();
        return VimAction::Continue;
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
            // Plain Enter inserts a newline (vim behaviour). Exit only via
            // Command mode (`:wq`/`:q`).
            let (t, i) = composer::insert_newline(&state.text, state.cursor);
            state.text = t;
            state.cursor = i;
            VimAction::Continue
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
    fn plain_enter_inserts_newline() {
        let mut s = VimState::new("ab".to_string());
        s.cursor = 1;
        assert_eq!(handle_insert(&mut s, enter()), VimAction::Continue);
        assert_eq!(s.text, "a\nb");
        assert_eq!(s.cursor, 2);
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
    fn ctrl_c_drops_to_normal() {
        let mut s = VimState::new("orig".to_string());
        s.text = "changed".to_string();
        s.cursor = 7;
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(handle_insert(&mut s, ctrl_c), VimAction::Continue);
        assert_eq!(s.mode, VimMode::Normal);
        // text retained (no discard), still modified
        assert_eq!(s.text, "changed");
        assert!(s.is_modified());
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
