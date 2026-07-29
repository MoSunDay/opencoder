//! Composer undo/redo state — pure functions, no OOP.
//!
//! Tracks input snapshots so Ctrl+Z / Ctrl+Y can step backwards and forwards
//! through edits. Consecutive char-insertions at the end of the buffer are
//! collapsed into a single undo step (the common "typing a word" case).

/// Maximum consecutive char-insertions collapsed into one undo step.
const COLLAPSE_THRESHOLD: usize = 3;

#[derive(Clone, Debug, Default)]
pub struct UndoState {
    undo: Vec<(String, usize)>,
    redo: Vec<(String, usize)>,
}

/// Create an `UndoState` seeded with the starting text + cursor.
pub fn init(text: &str, cursor: usize) -> UndoState {
    UndoState {
        undo: vec![(text.to_string(), cursor)],
        redo: vec![],
    }
}

/// Record a new state after an edit. When `is_char_insert` is true and the
/// last undo entry's text is a prefix of `text` (chars appended at end) with
/// a small growth, the last entry is replaced instead of pushing — collapsing
/// consecutive typing into one undo step.
pub fn snapshot(state: &mut UndoState, text: &str, cursor: usize, is_char_insert: bool) {
    state.redo.clear();
    if is_char_insert && state.undo.len() > 1 {
        if let Some(last) = state.undo.last() {
            if text.starts_with(last.0.as_str()) {
                let diff = text.chars().count().saturating_sub(last.0.chars().count());
                if diff <= COLLAPSE_THRESHOLD {
                    *state.undo.last_mut().unwrap() = (text.to_string(), cursor);
                    return;
                }
            }
        }
    }
    state.undo.push((text.to_string(), cursor));
}

/// Undo the last edit. The current state is pushed to the redo stack.
/// Returns the state to restore, or `None` if already at the initial state.
pub fn undo(state: &mut UndoState, current_text: &str, current_cursor: usize) -> Option<(String, usize)> {
    if state.undo.len() <= 1 {
        return None;
    }
    state.redo.push((current_text.to_string(), current_cursor));
    state.undo.pop();
    state.undo.last().cloned()
}

/// Redo the last undone edit. The current state is pushed to the undo stack.
/// Returns the state to restore, or `None` if there is nothing to redo.
pub fn redo(state: &mut UndoState, current_text: &str, current_cursor: usize) -> Option<(String, usize)> {
    let entry = state.redo.pop()?;
    state.undo.push((current_text.to_string(), current_cursor));
    Some(entry)
}

/// Reset to the given starting state, clearing all undo/redo history.
pub fn reset(state: &mut UndoState, text: &str, cursor: usize) {
    state.undo.clear();
    state.redo.clear();
    state.undo.push((text.to_string(), cursor));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_collapse_consecutive_chars() {
        let mut s = init("", 0);
        snapshot(&mut s, "a", 1, true);
        snapshot(&mut s, "ab", 2, true);
        snapshot(&mut s, "abc", 3, true);
        assert_eq!(s.undo.len(), 2); // initial "" + collapsed "abc"
    }

    #[test]
    fn undo_redo_basic() {
        let mut s = init("", 0);
        snapshot(&mut s, "hello", 5, true);
        assert_eq!(undo(&mut s, "hello", 5), Some(("".to_string(), 0)));
        assert_eq!(redo(&mut s, "", 0), Some(("hello".to_string(), 5)));
    }

    #[test]
    fn undo_at_initial_returns_none() {
        let mut s = init("", 0);
        assert!(undo(&mut s, "", 0).is_none());
    }

    #[test]
    fn snapshot_clears_redo() {
        let mut s = init("", 0);
        snapshot(&mut s, "a", 1, true);
        undo(&mut s, "a", 1);
        assert!(!s.redo.is_empty());
        snapshot(&mut s, "b", 1, true);
        assert!(s.redo.is_empty());
    }

    #[test]
    fn no_collapse_for_backspace() {
        let mut s = init("", 0);
        snapshot(&mut s, "abc", 3, true);
        snapshot(&mut s, "", 0, false); // backspace → not char insert
        assert_eq!(s.undo.len(), 3); // not collapsed
    }

    #[test]
    fn collapse_threshold() {
        let mut s = init("", 0);
        snapshot(&mut s, "a", 1, true);
        snapshot(&mut s, "ab", 2, true);
        snapshot(&mut s, "abc", 3, true);
        snapshot(&mut s, "abcd", 4, true);
        assert_eq!(s.undo.len(), 2); // initial "" + collapsed "abcd"
    }
}
