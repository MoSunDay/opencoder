//! Search (`/`, `?`, `n`, `N`) for the vim engine.
//!
//! [`find`] is a pure matcher returning a char index. [`handle_search`] drives
//! the in-progress search buffer (the line shown as `/query` or `?query` while
//! typing) and commits it on `Enter`, recording the last search so `n`/`N` in
//! Normal mode can repeat it.

use super::motion::{byte_offset_for_char, char_index_at_byte};
use super::state::{VimAction, VimMode};
use crossterm::event::{KeyCode, KeyEvent};

/// Find the next match of `query` in `text`. If `forward`, search starting one
/// char after `cursor`, wrapping to the start; otherwise search backward ending
/// one char before `cursor`, wrapping to the end. Returns the char index of the
/// match start, or `None` if `query` is empty / not found.
pub fn find(text: &str, cursor: usize, query: &str, forward: bool) -> Option<usize> {
    if query.is_empty() {
        return None;
    }
    if forward {
        let from_byte = {
            let after = cursor.saturating_add(1);
            byte_offset_for_char(text, after)
        };
        if let Some(rel) = text[from_byte..].find(query) {
            return Some(char_index_at_byte(text, from_byte + rel));
        }
        text.find(query).map(|b| char_index_at_byte(text, b))
    } else {
        let before_byte = byte_offset_for_char(text, cursor);
        if before_byte > 0 {
            if let Some(b) = text[..before_byte].rfind(query) {
                return Some(char_index_at_byte(text, b));
            }
        }
        text.rfind(query).map(|b| char_index_at_byte(text, b))
    }
}

pub fn handle_search(state: &mut super::state::VimState, k: KeyEvent) -> VimAction {
    match k.code {
        KeyCode::Esc => {
            state.mode = VimMode::Normal;
            state.search_input.clear();
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Backspace => {
            if state.search_input.is_empty() {
                state.mode = VimMode::Normal;
                state.reset_pending();
            } else {
                // drop last char
                let mut chars: Vec<char> = state.search_input.chars().collect();
                chars.pop();
                state.search_input = chars.into_iter().collect();
            }
            VimAction::Continue
        }
        KeyCode::Enter => {
            let q = state.search_input.clone();
            let fwd = state.search_forward;
            state.last_search = Some((q.clone(), fwd));
            if let Some(pos) = find(&state.text, state.cursor, &q, fwd) {
                state.cursor = pos;
                state.status.clear();
            } else {
                state.status = format!("Pattern not found: {}", q);
            }
            state.mode = VimMode::Normal;
            state.search_input.clear();
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char(c) if !c.is_control() => {
            state.search_input.push(c);
            VimAction::Continue
        }
        _ => VimAction::Continue,
    }
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
    fn backspace() -> KeyEvent {
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)
    }

    #[test]
    fn find_forward_basic() {
        let s = "foo bar foo";
        assert_eq!(find(s, 0, "foo", true), Some(8)); // skips current
        assert_eq!(find(s, 8, "foo", true), Some(0)); // wraps
        assert_eq!(find(s, 0, "bar", true), Some(4));
    }

    #[test]
    fn find_backward_basic() {
        let s = "foo bar foo";
        assert_eq!(find(s, 11, "foo", false), Some(8));
        assert_eq!(find(s, 8, "foo", false), Some(0));
        assert_eq!(find(s, 0, "foo", false), Some(8)); // wraps to end
    }

    #[test]
    fn find_not_found_and_empty() {
        assert_eq!(find("abc", 0, "xyz", true), None);
        assert_eq!(find("abc", 0, "", true), None);
        assert_eq!(find("abc", 0, "xyz", false), None);
        assert_eq!(find("abc", 0, "", false), None);
    }

    #[test]
    fn find_overlapping_matches() {
        // "ana" in "banana": forward from 0 lands on index 1.
        let s = "banana";
        assert_eq!(find(s, 0, "ana", true), Some(1));
        // from cursor 1 (on the first match), the next match is at idx 3
        // (overlapping matches are found as the search advances one char).
        assert_eq!(find(s, 1, "ana", true), Some(3));
    }

    #[test]
    fn find_multibyte() {
        let s = "héllo héy";
        // 'é' is at char idx 1 and 7.
        assert_eq!(find(s, 0, "é", true), Some(1));
        assert_eq!(find(s, 1, "é", true), Some(7));
    }

    #[test]
    fn handle_search_esc_cancels() {
        let mut s = VimState::new("abc".to_string());
        s.mode = VimMode::Search;
        s.search_input = "x".to_string();
        assert_eq!(handle_search(&mut s, esc()), VimAction::Continue);
        assert_eq!(s.mode, VimMode::Normal);
        assert_eq!(s.search_input, "");
    }

    #[test]
    fn handle_search_enter_records_and_moves() {
        let mut s = VimState::new("foo bar foo".to_string());
        s.mode = VimMode::Search;
        s.search_forward = true;
        s.cursor = 0;
        for ch in "foo".chars() {
            handle_search(&mut s, k(ch));
        }
        assert_eq!(handle_search(&mut s, enter()), VimAction::Continue);
        assert_eq!(s.mode, VimMode::Normal);
        assert_eq!(s.cursor, 8); // lands on the second "foo"
        assert_eq!(s.last_search, Some(("foo".to_string(), true)));
    }

    #[test]
    fn handle_search_enter_not_found_sets_status() {
        let mut s = VimState::new("abc".to_string());
        s.mode = VimMode::Search;
        s.search_forward = true;
        s.search_input = "zzz".to_string();
        handle_search(&mut s, enter());
        assert!(s.status.contains("Pattern not found"));
        assert_eq!(s.last_search, Some(("zzz".to_string(), true)));
    }

    #[test]
    fn handle_search_backspace_on_empty_cancels() {
        let mut s = VimState::new("abc".to_string());
        s.mode = VimMode::Search;
        s.search_input.clear();
        assert_eq!(handle_search(&mut s, backspace()), VimAction::Continue);
        assert_eq!(s.mode, VimMode::Normal);
    }

    #[test]
    fn handle_search_backspace_pops_char() {
        let mut s = VimState::new("abc".to_string());
        s.mode = VimMode::Search;
        s.search_input = "ab".to_string();
        handle_search(&mut s, backspace());
        assert_eq!(s.search_input, "a");
        assert_eq!(s.mode, VimMode::Search);
    }
}
