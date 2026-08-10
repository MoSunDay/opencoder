//! Vim-style undo/redo for the shared vim engine (notepad + plan editor).
//!
//! Normal mode `u` undoes the last edit and `Ctrl+R` redoes it. An entire
//! Insert session (from the insert-entry key until `Esc`/`Ctrl+C`) collapses
//! into a single undo step, matching vim semantics. Pure functions over
//! [`UndoHistory`] — no I/O, no OOP.

use super::state::{VimAction, VimMode, VimState};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Maximum undo steps kept (excluding the initial snapshot).
const MAX_HISTORY: usize = 100;

/// Double-stack undo/redo plus the insert-session tracking flags.
#[derive(Clone, Debug, Default)]
pub struct UndoHistory {
    pub undo: Vec<(String, usize)>,
    pub redo: Vec<(String, usize)>,
    /// True while the engine is inside an Insert session.
    pub insert_session: bool,
    /// True once the current session's snapshot has been recorded.
    pub session_recorded: bool,
}

/// Seed a fresh history with the starting text + cursor as the undo floor.
pub fn init(text: &str, cursor: usize) -> UndoHistory {
    UndoHistory {
        undo: vec![(text.to_string(), cursor)],
        redo: vec![],
        insert_session: false,
        session_recorded: false,
    }
}

/// Push a change snapshot (state before an edit). A new edit invalidates the
/// redo stack; history is capped at [`MAX_HISTORY`] steps by dropping the
/// oldest non-initial entry.
fn record_change(history: &mut UndoHistory, before_text: &str, before_cursor: usize) {
    history.redo.clear();
    history.undo.push((before_text.to_string(), before_cursor));
    if history.undo.len() > MAX_HISTORY + 1 {
        history.undo.remove(1);
    }
}

/// Undo up to `count` steps. Each step pushes the current state onto the redo
/// stack. Returns the state to restore, or `None` when already at the initial
/// snapshot.
pub fn undo(
    history: &mut UndoHistory,
    text: &str,
    cursor: usize,
    count: usize,
) -> Option<(String, usize)> {
    let mut current = (text.to_string(), cursor);
    let mut restored = None;
    for _ in 0..count.max(1) {
        if history.undo.len() <= 1 {
            break;
        }
        history.redo.push(current);
        current = history.undo.pop().unwrap();
        restored = Some(current.clone());
    }
    restored
}

/// Redo up to `count` undone steps (the inverse of [`undo`]).
pub fn redo(
    history: &mut UndoHistory,
    text: &str,
    cursor: usize,
    count: usize,
) -> Option<(String, usize)> {
    let mut current = (text.to_string(), cursor);
    let mut restored = None;
    for _ in 0..count.max(1) {
        let entry = match history.redo.pop() {
            Some(e) => e,
            None => break,
        };
        history.undo.push(current);
        current = entry;
        restored = Some(current.clone());
    }
    restored
}

/// Post-dispatch hook called by `handle_vim_key` after the active mode handled
/// a key. Records the pre-edit snapshot when the text changed and manages the
/// insert-session boundaries (session start on Normal→Insert transitions, end
/// on Esc/Ctrl+C). `action == VimAction::Exit` (e.g. `:q!`, which restores the
/// original text) never records.
pub fn after_dispatch(
    state: &mut VimState,
    before_text: &str,
    before_cursor: usize,
    action: VimAction,
) {
    if action == VimAction::Exit {
        return;
    }
    let changed = state.text != before_text;
    let now_insert = state.mode == VimMode::Insert;
    let h = &mut state.history;
    if now_insert && !h.insert_session {
        // A Normal-mode key entered Insert (i/a/o/c/I/A/O/C/...). The session
        // snapshot is the pre-key state; when that key also mutated the text
        // (o/O/c/C) it is recorded immediately.
        h.insert_session = true;
        if changed {
            record_change(h, before_text, before_cursor);
            h.session_recorded = true;
        } else {
            h.session_recorded = false;
        }
        return;
    }
    if h.insert_session && !now_insert {
        // Esc / Ctrl+C left the session. Fall through so a same-key text
        // change (never produced by Esc/Ctrl+C) would still be recorded.
        h.insert_session = false;
        h.session_recorded = false;
    }
    if h.insert_session {
        // Inside a session only the first text change is recorded; subsequent
        // characters, backspaces and newlines collapse into that one step.
        if changed && !h.session_recorded {
            record_change(h, before_text, before_cursor);
            h.session_recorded = true;
        }
        return;
    }
    if changed {
        record_change(h, before_text, before_cursor);
    }
}

/// Intercept Normal-mode `u` (undo) / `Ctrl+R` (redo) before normal dispatch.
/// Returns `Some(action)` when handled; the caller must skip dispatch.
/// Only plain Normal mode without a pending operator or `g`-sequence is
/// intercepted, so `u` still types in Insert/Command/Search and `du` stays a
/// (cancelled) operator sequence.
pub fn maybe_handle_key(state: &mut VimState, k: &KeyEvent) -> Option<VimAction> {
    if state.mode != VimMode::Normal || state.pending_op.is_some() || state.pending_g {
        return None;
    }
    let undo_key =
        matches!(k.code, KeyCode::Char('u')) && !k.modifiers.contains(KeyModifiers::CONTROL);
    let redo_key =
        matches!(k.code, KeyCode::Char('r')) && k.modifiers.contains(KeyModifiers::CONTROL);
    if !undo_key && !redo_key {
        return None;
    }
    let count = state.count.unwrap_or(1);
    state.reset_pending();
    let restored = if undo_key {
        undo(&mut state.history, &state.text, state.cursor, count)
    } else {
        redo(&mut state.history, &state.text, state.cursor, count)
    };
    if let Some((text, cursor)) = restored {
        state.text = text;
        state.cursor = cursor;
        state.clamp_cursor();
    }
    Some(VimAction::Continue)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vim::handle_vim_key;

    fn k(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }
    fn esc() -> KeyEvent {
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
    }
    fn backspace() -> KeyEvent {
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)
    }
    const W: u16 = 80;
    const P: u16 = 2;

    fn press(s: &mut VimState, key: KeyEvent) {
        handle_vim_key(s, key, W, P);
    }
    fn type_str(s: &mut VimState, t: &str) {
        for c in t.chars() {
            press(s, k(c));
        }
    }
    fn norm(text: &str, cursor: usize) -> VimState {
        let mut s = VimState::new(text.to_string());
        s.mode = VimMode::Normal;
        s.cursor = cursor;
        s
    }

    #[test]
    fn insert_session_undoes_whole_session() {
        // i + "ab" + Esc → one `u` restores the pre-session text.
        let mut s = norm("hi", 2);
        press(&mut s, k('i'));
        type_str(&mut s, "ab");
        press(&mut s, esc());
        assert_eq!(s.text, "hiab");
        press(&mut s, k('u'));
        assert_eq!(s.text, "hi");
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn o_session_is_one_step() {
        let mut s = norm("ab", 1);
        press(&mut s, k('o')); // opens line below and enters Insert
        type_str(&mut s, "x");
        press(&mut s, esc());
        assert_eq!(s.text, "ab\nx");
        press(&mut s, k('u'));
        assert_eq!(s.text, "ab");
        assert_eq!(s.cursor, 1);
    }

    #[test]
    fn cw_session_is_one_step() {
        let mut s = norm("foo bar", 0);
        press(&mut s, k('c'));
        press(&mut s, k('w')); // deletes "foo " and enters Insert
        type_str(&mut s, "baz ");
        press(&mut s, esc());
        assert_eq!(s.text, "baz bar");
        press(&mut s, k('u'));
        assert_eq!(s.text, "foo bar");
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn backspace_inside_session_collapses() {
        let mut s = norm("hi", 2);
        press(&mut s, k('i'));
        type_str(&mut s, "ab");
        press(&mut s, backspace()); // "hia"
        type_str(&mut s, "c"); // "hiac"
        press(&mut s, esc());
        assert_eq!(s.text, "hiac");
        press(&mut s, k('u'));
        assert_eq!(s.text, "hi");
    }

    #[test]
    fn x_undoes_one_edit_per_u() {
        let mut s = norm("abc", 0);
        press(&mut s, k('x'));
        press(&mut s, k('x'));
        assert_eq!(s.text, "c");
        press(&mut s, k('u'));
        assert_eq!(s.text, "bc");
        press(&mut s, k('u'));
        assert_eq!(s.text, "abc");
        press(&mut s, k('u')); // at the floor: no-op
        assert_eq!(s.text, "abc");
    }

    #[test]
    fn count_undo_undoes_multiple_steps() {
        let mut s = norm("abcdef", 0);
        for _ in 0..3 {
            press(&mut s, k('x'));
        }
        assert_eq!(s.text, "def");
        press(&mut s, k('3'));
        press(&mut s, k('u'));
        assert_eq!(s.text, "abcdef");
    }

    #[test]
    fn ctrl_r_redoes_undone_edit() {
        let mut s = norm("abc", 0);
        press(&mut s, k('x'));
        assert_eq!(s.text, "bc");
        press(&mut s, k('u'));
        assert_eq!(s.text, "abc");
        press(&mut s, ctrl('r'));
        assert_eq!(s.text, "bc");
    }

    #[test]
    fn count_redo_redoes_multiple_steps() {
        let mut s = norm("abcdef", 0);
        for _ in 0..2 {
            press(&mut s, k('x'));
        }
        for _ in 0..2 {
            press(&mut s, k('u'));
        }
        assert_eq!(s.text, "abcdef");
        press(&mut s, k('2'));
        press(&mut s, ctrl('r'));
        assert_eq!(s.text, "cdef");
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut s = norm("abc", 0);
        press(&mut s, k('x'));
        press(&mut s, k('u'));
        assert!(!s.history.redo.is_empty());
        press(&mut s, k('x')); // new edit invalidates redo
        assert!(s.history.redo.is_empty());
        press(&mut s, ctrl('r'));
        assert_eq!(s.text, "bc"); // nothing to redo — unchanged
    }

    #[test]
    fn undo_at_initial_is_noop() {
        let mut s = norm("abc", 0);
        press(&mut s, k('u'));
        assert_eq!(s.text, "abc");
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn u_types_normally_in_insert() {
        let mut s = VimState::new("hi".to_string()); // starts in Insert
        press(&mut s, k('u'));
        assert_eq!(s.text, "hiu");
    }

    #[test]
    fn u_types_into_cmdline_in_command_mode() {
        let mut s = VimState::new("hi".to_string());
        s.mode = VimMode::Command;
        press(&mut s, k('u'));
        assert_eq!(s.cmdline, "u");
        assert_eq!(s.text, "hi");
    }

    #[test]
    fn u_types_into_search_input() {
        let mut s = VimState::new("hi".to_string());
        s.mode = VimMode::Search;
        press(&mut s, k('u'));
        assert_eq!(s.search_input, "u");
    }

    #[test]
    fn pending_op_defers_u() {
        let mut s = norm("abc", 0);
        press(&mut s, k('d'));
        press(&mut s, k('u')); // unknown motion → cancels the operator
        assert_eq!(s.text, "abc");
        assert_eq!(s.pending_op, None);
        assert_eq!(s.history.undo.len(), 1); // nothing recorded
    }

    #[test]
    fn history_caps_at_100_steps() {
        let mut s = norm("", 0);
        for i in 0..105usize {
            let c = (b'a' + (i % 26) as u8) as char;
            press(&mut s, k('i'));
            press(&mut s, k(c));
            press(&mut s, esc());
        }
        assert_eq!(s.history.undo.len(), 101); // initial + 100 steps
    }
}
