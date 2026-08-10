//! Pure vim-mode editing engine for the TUI plan editor.
//!
//! Operates on a [`VimState`] (text + cursor + mode + small pending-state).
//! All movement reuses `crate::composer` wrapping math so the cursor never
//! diverges from the renderer. The engine performs no I/O; the caller decides
//! persistence on [`VimAction::Exit`] based on [`VimState::is_modified`].
//!
//! Modes: Normal (navigate/operators), Insert (type), Command-line (`:`),
//! Search (`/` `?` + `n`/`N`). Exits: `:q!`/`:q` discard (restore original);
//! `:wq` save.

pub mod command;
pub mod insert;
pub mod motion;
pub mod normal;
pub mod search;
pub mod state;
pub mod undo;

mod actions;
mod ops;

pub use state::{VimAction, VimMode, VimState};

use crossterm::event::KeyEvent;

/// Dispatch a key to the active mode. Returns [`VimAction::Exit`] when the user
/// wants to leave the editor (the caller then checks `is_modified` to decide
/// whether to persist). On a discard exit the engine has already restored
/// `text` to `original`, so `is_modified` will be `false`.
pub fn handle_vim_key(state: &mut VimState, k: KeyEvent, inner_w: u16, prompt_w: u16) -> VimAction {
    // Normal-mode `u` / `Ctrl+R` (undo/redo) are intercepted before dispatch.
    if let Some(action) = undo::maybe_handle_key(state, &k) {
        return action;
    }
    let before_text = state.text.clone();
    let before_cursor = state.cursor;
    let action = match state.mode {
        VimMode::Insert => insert::handle_insert(state, k),
        VimMode::Normal => normal::handle_normal(state, k, inner_w, prompt_w),
        VimMode::Command => command::handle_command(state, k),
        VimMode::Search => search::handle_search(state, k),
    };
    // Snapshot diff: record the pre-key state when the text changed and keep
    // insert-session boundaries so an entire session undoes as one step.
    undo::after_dispatch(state, &before_text, before_cursor, action);
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vim::state::VimMode;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn k(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
    fn esc() -> KeyEvent {
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
    }
    fn enter() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
    }
    fn shift_enter() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)
    }

    fn iw() -> (u16, u16) {
        (80, 2)
    }

    #[test]
    fn type_then_wq_preserves_text() {
        // Start in Insert with empty text, type "hello", Esc, :wq.
        let (w, p) = iw();
        let mut s = VimState::new(String::new());
        for ch in "hello".chars() {
            assert_eq!(handle_vim_key(&mut s, k(ch), w, p), VimAction::Continue);
        }
        assert_eq!(handle_vim_key(&mut s, esc(), w, p), VimAction::Continue);
        assert_eq!(s.mode, VimMode::Normal);
        for ch in ":wq".chars() {
            handle_vim_key(&mut s, k(ch), w, p);
        }
        assert_eq!(handle_vim_key(&mut s, enter(), w, p), VimAction::Exit);
        assert_eq!(s.text, "hello");
        assert!(s.is_modified());
    }

    #[test]
    fn q_discards_edits() {
        let (w, p) = iw();
        let mut s = VimState::new("orig".to_string());
        // Enter Insert by pressing Esc then 'i'? We start in Insert already.
        for ch in "XYZ".chars() {
            handle_vim_key(&mut s, k(ch), w, p);
        }
        handle_vim_key(&mut s, esc(), w, p);
        for ch in ":q!".chars() {
            handle_vim_key(&mut s, k(ch), w, p);
        }
        assert_eq!(handle_vim_key(&mut s, enter(), w, p), VimAction::Exit);
        assert_eq!(s.text, "orig");
        assert!(!s.is_modified());
    }

    #[test]
    fn search_moves_cursor_and_n_repeats() {
        let (w, p) = iw();
        let mut s = VimState::new("foo world bar world".to_string());
        // Go to Normal first.
        handle_vim_key(&mut s, esc(), w, p);
        assert_eq!(s.mode, VimMode::Normal);
        s.cursor = 0;
        handle_vim_key(&mut s, k('/'), w, p);
        for ch in "world".chars() {
            handle_vim_key(&mut s, k(ch), w, p);
        }
        assert_eq!(handle_vim_key(&mut s, enter(), w, p), VimAction::Continue);
        assert_eq!(s.cursor, 4); // first "world"
        handle_vim_key(&mut s, k('n'), w, p);
        assert_eq!(s.cursor, 14); // second "world"
        handle_vim_key(&mut s, k('N'), w, p);
        assert_eq!(s.cursor, 4); // back to first
    }

    #[test]
    fn dw_deletes_word_normal_flow() {
        let (w, p) = iw();
        let mut s = VimState::new("hello world".to_string());
        handle_vim_key(&mut s, esc(), w, p);
        s.cursor = 0;
        handle_vim_key(&mut s, k('d'), w, p);
        handle_vim_key(&mut s, k('w'), w, p);
        assert_eq!(s.text, "world");
    }

    #[test]
    fn dd_then_p_restores_line() {
        let (w, p) = iw();
        let mut s = VimState::new("line1\nline2\nline3".to_string());
        handle_vim_key(&mut s, esc(), w, p);
        s.cursor = 6; // on line2
        handle_vim_key(&mut s, k('d'), w, p);
        handle_vim_key(&mut s, k('d'), w, p);
        assert_eq!(s.text, "line1\nline3");
        // paste below line1: cursor on line1
        s.cursor = 0;
        handle_vim_key(&mut s, k('p'), w, p);
        assert_eq!(s.text, "line1\nline2\nline3");
    }

    #[test]
    fn count_2x_deletes_two() {
        let (w, p) = iw();
        let mut s = VimState::new("abcdef".to_string());
        handle_vim_key(&mut s, esc(), w, p);
        s.cursor = 0;
        handle_vim_key(&mut s, k('2'), w, p);
        handle_vim_key(&mut s, k('x'), w, p);
        assert_eq!(s.text, "cdef");
    }

    #[test]
    fn count_3l_moves_three() {
        let (w, p) = iw();
        let mut s = VimState::new("abcdef".to_string());
        handle_vim_key(&mut s, esc(), w, p);
        s.cursor = 0;
        handle_vim_key(&mut s, k('3'), w, p);
        handle_vim_key(&mut s, k('l'), w, p);
        assert_eq!(s.cursor, 3);
    }

    #[test]
    fn o_opens_line_below_and_inserts() {
        let (w, p) = iw();
        let mut s = VimState::new("ab".to_string());
        handle_vim_key(&mut s, esc(), w, p);
        s.cursor = 1;
        handle_vim_key(&mut s, k('o'), w, p);
        assert_eq!(s.mode, VimMode::Insert);
        assert_eq!(s.text, "ab\n");
        assert_eq!(s.cursor, 3);
        handle_vim_key(&mut s, k('x'), w, p);
        assert_eq!(s.text, "ab\nx");
        handle_vim_key(&mut s, esc(), w, p);
        assert_eq!(s.mode, VimMode::Normal);
        assert_eq!(s.cursor, 3); // 'x'
    }

    #[test]
    fn newline_chord_in_insert_via_top_dispatcher() {
        let (w, p) = iw();
        let mut s = VimState::new("ab".to_string());
        s.cursor = 1;
        handle_vim_key(&mut s, shift_enter(), w, p);
        assert_eq!(s.text, "a\nb");
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn full_dispatch_switches_modes() {
        let (w, p) = iw();
        let mut s = VimState::new("hi".to_string());
        // Insert -> Esc -> Normal -> ':' -> Command -> Esc -> Normal
        handle_vim_key(&mut s, esc(), w, p);
        assert_eq!(s.mode, VimMode::Normal);
        handle_vim_key(&mut s, k(':'), w, p);
        assert_eq!(s.mode, VimMode::Command);
        handle_vim_key(&mut s, esc(), w, p);
        assert_eq!(s.mode, VimMode::Normal);
        handle_vim_key(&mut s, k('/'), w, p);
        assert_eq!(s.mode, VimMode::Search);
        handle_vim_key(&mut s, esc(), w, p);
        assert_eq!(s.mode, VimMode::Normal);
    }
}

// Unit tests for the plain-command dispatch (dispatch_plain). These live in
// mod.rs rather than actions.rs to keep actions.rs under the line limit; they
// drive `dispatch_plain` directly with an explicit count.
#[cfg(test)]
mod actions_tests {
    use crate::vim::actions::dispatch_plain;
    use crate::vim::state::{VimMode, VimState};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn k(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
    fn norm(text: &str, cursor: usize) -> VimState {
        let mut s = VimState::new(text.to_string());
        s.mode = VimMode::Normal;
        s.cursor = cursor;
        s
    }
    fn disp(state: &mut VimState, key: KeyEvent) {
        dispatch_plain(state, key, 1, state.char_count(), 80, 2);
    }

    #[test]
    fn hjkl_and_count() {
        let mut s = norm("abcdef", 0);
        dispatch_plain(&mut s, k('l'), 1, 6, 80, 2);
        assert_eq!(s.cursor, 1);
        dispatch_plain(&mut s, k('l'), 3, 6, 80, 2);
        assert_eq!(s.cursor, 4);
        dispatch_plain(&mut s, k('h'), 1, 6, 80, 2);
        assert_eq!(s.cursor, 3);
    }

    #[test]
    fn word_motions() {
        let mut s = norm("foo bar baz", 0);
        disp(&mut s, k('w'));
        assert_eq!(s.cursor, 4);
        disp(&mut s, k('b'));
        assert_eq!(s.cursor, 0);
        disp(&mut s, k('e'));
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn zero_caret_dollar() {
        let mut s = norm("  foo bar", 5);
        disp(&mut s, k('0'));
        assert_eq!(s.cursor, 0);
        disp(&mut s, k('^'));
        assert_eq!(s.cursor, 2);
        disp(&mut s, k('$'));
        assert_eq!(s.cursor, 8);
    }

    #[test]
    fn gg_and_big_g() {
        let mut s = norm("a\nb\nc", 0);
        disp(&mut s, k('G'));
        assert_eq!(s.cursor, 4);
        disp(&mut s, k('g'));
        disp(&mut s, k('g'));
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn insert_entries() {
        let mut s = norm("abc", 0);
        disp(&mut s, k('i'));
        assert_eq!(s.mode, VimMode::Insert);
        assert_eq!(s.cursor, 0);
        let mut s = norm("abc", 0);
        disp(&mut s, k('a'));
        assert_eq!(s.cursor, 1);
        let mut s = norm("abc", 0);
        disp(&mut s, k('A'));
        assert_eq!(s.cursor, 3);
    }

    #[test]
    fn o_and_big_o_open_lines() {
        let mut s = norm("ab", 1);
        disp(&mut s, k('o'));
        assert_eq!(s.text, "ab\n");
        assert_eq!(s.cursor, 3);
        let mut s = norm("ab", 1);
        disp(&mut s, k('O'));
        assert_eq!(s.text, "\nab");
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn x_and_big_x() {
        let mut s = norm("abcdef", 2);
        disp(&mut s, k('x'));
        assert_eq!(s.text, "abdef");
        assert_eq!(s.register, "c");
        let mut s = norm("abcdef", 3);
        dispatch_plain(&mut s, k('x'), 2, 6, 80, 2);
        assert_eq!(s.text, "abcf");
        let mut s = norm("abcdef", 3);
        disp(&mut s, k('X'));
        assert_eq!(s.text, "abdef");
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn big_d_and_big_c() {
        let mut s = norm("foo bar", 0);
        disp(&mut s, k('D'));
        assert_eq!(s.text, "");
        assert_eq!(s.register, "foo bar");
        let mut s = norm("foo bar", 0);
        disp(&mut s, k('C'));
        assert_eq!(s.text, "");
        assert_eq!(s.mode, VimMode::Insert);
    }

    #[test]
    fn charwise_paste_p_and_big_p() {
        let mut s = norm("abc", 0);
        s.register = "XY".to_string();
        s.register_linewise = false;
        disp(&mut s, k('p'));
        assert_eq!(s.text, "aXYbc");
        assert_eq!(s.cursor, 2);
        let mut s = norm("abc", 1);
        s.register = "Z".to_string();
        s.register_linewise = false;
        disp(&mut s, k('P'));
        assert_eq!(s.text, "aZbc");
    }

    #[test]
    fn linewise_paste_restores_line() {
        let mut s = norm("a\nc", 0);
        s.register = "b\n".to_string();
        s.register_linewise = true;
        disp(&mut s, k('p'));
        assert_eq!(s.text, "a\nb\nc");
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn search_repeat_n_big_n() {
        let mut s = norm("foo bar foo", 0);
        s.last_search = Some(("foo".to_string(), true));
        disp(&mut s, k('n'));
        assert_eq!(s.cursor, 8);
        disp(&mut s, k('N'));
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn mode_switches() {
        let mut s = norm("abc", 0);
        disp(&mut s, k(':'));
        assert_eq!(s.mode, VimMode::Command);
        let mut s = norm("abc", 0);
        disp(&mut s, k('/'));
        assert_eq!(s.mode, VimMode::Search);
        assert!(s.search_forward);
        let mut s = norm("abc", 0);
        disp(&mut s, k('?'));
        assert!(!s.search_forward);
    }

    #[test]
    fn operator_pending_set() {
        let mut s = norm("abc", 0);
        disp(&mut s, k('d'));
        assert_eq!(s.pending_op, Some('d'));
        disp(&mut s, k('c'));
        assert_eq!(s.pending_op, Some('c'));
    }

    #[test]
    fn empty_register_paste_is_noop() {
        let mut s = norm("abc", 1);
        s.register.clear();
        disp(&mut s, k('p'));
        assert_eq!(s.text, "abc");
        assert_eq!(s.cursor, 1);
    }
}
