//! Normal-mode entry point for the vim engine.
//!
//! [`handle_normal`] runs the prefix pipeline (Ctrl+C no-op, readline
//! shortcuts, count digit prefix, Enter no-op) and then either dispatches
//! a pending operator ([`super::ops::apply_operator`]) or a plain command
//! ([`super::actions::dispatch_plain`]). Exiting the editor is only possible
//! from Command mode via `:q`/`:q!`/`:wq`.

use super::actions::dispatch_plain;
use super::motion::line_start;
use super::ops::apply_operator;
use super::state::{is_ctrl_c, VimAction, VimState};
use crate::composer;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn handle_normal(state: &mut VimState, k: KeyEvent, inner_w: u16, prompt_w: u16) -> VimAction {
    // 1. Ctrl+C: no-op (intercepted so it is not treated as the `c` operator).
    //    Exit only via Command mode (`:q`/`:q!`/`:wq`).
    if is_ctrl_c(&k) {
        state.reset_pending();
        return VimAction::Continue;
    }
    // 2. Readline shortcuts (shared with the composer input box).
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        match k.code {
            KeyCode::Char('a') => {
                state.cursor = 0;
                state.reset_pending();
                return VimAction::Continue;
            }
            KeyCode::Char('e') => {
                state.cursor = state.char_count();
                state.reset_pending();
                return VimAction::Continue;
            }
            KeyCode::Char('w') => {
                if let Some((t, i)) = composer::delete_word_back(&state.text, state.cursor) {
                    state.text = t;
                    state.cursor = i;
                }
                state.reset_pending();
                return VimAction::Continue;
            }
            _ => {}
        }
    }
    // 3. Digit prefix (but `0` right after an operator is the d0 motion).
    if let KeyCode::Char(c) = k.code {
        if c.is_ascii_digit() && !(state.pending_op.is_some() && c == '0' && state.count.is_none())
        {
            if c == '0' && state.count.is_none() {
                state.cursor = line_start(&state.text, state.cursor);
                state.reset_pending();
                return VimAction::Continue;
            }
            let d = (c as u8 - b'0') as usize;
            state.count = Some(state.count.unwrap_or(0) * 10 + d);
            return VimAction::Continue;
        }
    }
    // 4. Enter: no-op (exit only via Command mode).
    // 5. Operator pending: this key is the motion.
    if let Some(op) = state.pending_op {
        let count = state.count.unwrap_or(1);
        return apply_operator(state, op, k, count, inner_w, prompt_w);
    }
    // 6. Plain dispatch.
    let count = state.count.unwrap_or(1);
    let total = state.char_count();
    dispatch_plain(state, k, count, total, inner_w, prompt_w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vim::state::{VimMode, VimState};
    use crossterm::event::KeyModifiers;

    fn k(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
    fn esc() -> KeyEvent {
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
    }
    fn enter() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
    }

    fn norm(text: &str, cursor: usize) -> VimState {
        let mut s = VimState::new(text.to_string());
        s.mode = VimMode::Normal;
        s.cursor = cursor;
        s
    }

    #[test]
    fn ctrl_c_is_noop() {
        let mut s = norm("orig", 0);
        s.text = "changed".to_string();
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(handle_normal(&mut s, ctrl_c, 80, 2), VimAction::Continue);
        // text unchanged, still modified
        assert_eq!(s.text, "changed");
        assert!(s.is_modified());
    }

    #[test]
    fn count_accumulates_then_consumed() {
        let mut s = norm("abcdef", 0);
        handle_normal(&mut s, k('3'), 80, 2); // count=3
        handle_normal(&mut s, k('l'), 80, 2); // move 3
        assert_eq!(s.cursor, 3);
        assert_eq!(s.count, None); // consumed
    }

    #[test]
    fn zero_with_no_count_is_first_column() {
        let mut s = norm("  abc", 4);
        handle_normal(&mut s, k('0'), 80, 2);
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn zero_after_count_extends_count() {
        // "10l" moves 10 (clamped). count 1, then 0, then l.
        let mut s = norm("abc", 0);
        handle_normal(&mut s, k('1'), 80, 2);
        handle_normal(&mut s, k('0'), 80, 2);
        handle_normal(&mut s, k('l'), 80, 2);
        assert_eq!(s.cursor, 3); // clamped to end
    }

    #[test]
    fn d0_uses_zero_as_motion_not_digit() {
        let mut s = norm("foo bar", 4);
        handle_normal(&mut s, k('d'), 80, 2);
        handle_normal(&mut s, k('0'), 80, 2); // motion, not digit
        assert_eq!(s.text, "bar");
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn esc_resets_pending() {
        let mut s = norm("abc", 0);
        s.count = Some(5);
        s.pending_op = Some('d');
        handle_normal(&mut s, esc(), 80, 2);
        assert_eq!(s.count, None);
        assert_eq!(s.pending_op, None);
    }

    #[test]
    fn plain_enter_is_noop() {
        let mut s = norm("abc", 0);
        assert_eq!(handle_normal(&mut s, enter(), 80, 2), VimAction::Continue);
        assert_eq!(s.text, "abc");
    }

    #[test]
    fn ctrl_a_e_w_readline() {
        let mut s = norm("foo bar", 7);
        let ctrl_a = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
        handle_normal(&mut s, ctrl_a, 80, 2);
        assert_eq!(s.cursor, 0);
        let ctrl_e = KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL);
        handle_normal(&mut s, ctrl_e, 80, 2);
        assert_eq!(s.cursor, 7);
        let ctrl_w = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL);
        handle_normal(&mut s, ctrl_w, 80, 2);
        assert_eq!(s.text, "foo ");
    }

    #[test]
    fn count_gg_jumps_to_line() {
        let mut s = norm("a\nb\nc\nd", 0);
        handle_normal(&mut s, k('3'), 80, 2);
        handle_normal(&mut s, k('g'), 80, 2);
        handle_normal(&mut s, k('g'), 80, 2);
        assert_eq!(s.cursor, 4); // line 3 = "c"
    }

    #[test]
    fn count_big_g_jumps_to_line() {
        let mut s = norm("a\nb\nc", 0);
        handle_normal(&mut s, k('2'), 80, 2);
        handle_normal(&mut s, k('G'), 80, 2);
        assert_eq!(s.cursor, 2); // line 2 = "b"
    }

    #[test]
    fn unknown_key_resets_pending() {
        let mut s = norm("abc", 0);
        s.count = Some(2);
        handle_normal(&mut s, k('z'), 80, 2);
        assert_eq!(s.count, None);
    }
}
