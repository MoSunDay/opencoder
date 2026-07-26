//! Operator machinery for Normal mode: `d`, `c`, `y` combined with motions.
//!
//! [`apply_operator`] consumes the motion key following a pending operator and
//! performs the resulting delete / change / yank over the computed char range.
//! Line-range helpers (`dd`, `cc`, `yy`, `dG`, `dgg`) live here too, along with
//! the shared [`delete_range`] primitive.

use super::motion::{
    line_end_exclusive, line_start, line_start_by_number, word_backward, word_end, word_forward,
};
use super::state::{VimAction, VimMode, VimState};
use crossterm::event::{KeyCode, KeyEvent};

/// Delete a half-open char range, returning (new_text, removed).
pub(super) fn delete_range(text: &str, start: usize, end: usize) -> (String, String) {
    let chars: Vec<char> = text.chars().collect();
    let s = start.min(chars.len());
    let e = end.min(chars.len()).max(s);
    let removed: String = chars[s..e].iter().collect();
    let mut out: String = chars[..s].iter().collect();
    out.extend(chars[e..].iter());
    (out, removed)
}

/// 1-based line number of the line containing `cursor`.
fn current_line_number(text: &str, cursor: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let c = cursor.min(chars.len());
    1 + (0..c).filter(|&i| chars[i] == '\n').count()
}

/// Char range covering `count` lines starting at the cursor's line. Includes the
/// trailing newline of the last covered line; if the group runs to EOF the
/// preceding newline is absorbed so no dangling blank line remains. Used by
/// `dd`/`yy`.
fn linewise_range(text: &str, cursor: usize, count: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let cur_line = current_line_number(text, cursor);
    let mut start = line_start_by_number(text, cur_line);
    let mut end = start;
    let mut seen = 0usize;
    let mut i = start;
    while i < n {
        if chars[i] == '\n' {
            seen += 1;
            if seen == count {
                end = i + 1;
                break;
            }
        }
        i += 1;
    }
    if seen < count {
        end = n;
        if start > 0 && chars[start - 1] == '\n' {
            start -= 1; // absorb preceding newline through EOF
        }
    }
    (start, end)
}

/// Char range [0, start-of-next-line-after-current) - lines 1..current. `dgg`.
fn linewise_range_to_current(text: &str, cursor: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let cur_end = line_end_exclusive(text, cursor);
    let end = if cur_end < n { cur_end + 1 } else { n };
    (0, end)
}

/// Resolve an operator's motion key into (start, end, linewise). `None` cancels
/// the operator.
fn operator_motion(
    text: &str,
    cursor: usize,
    op: char,
    k: &KeyEvent,
    count: usize,
) -> Option<(usize, usize, bool)> {
    let total = text.chars().count();
    match k.code {
        KeyCode::Char(c) if c == op => {
            // linewise self: dd / cc / yy
            if op == 'c' {
                // keep one empty line: content only, newline preserved
                let first = line_start(text, cursor);
                let cur_line = current_line_number(text, cursor);
                let last_line = cur_line + count.saturating_sub(1);
                let last_start = line_start_by_number(text, last_line);
                let last_end = line_end_exclusive(text, last_start);
                Some((first, last_end, true))
            } else {
                let (s, e) = linewise_range(text, cursor, count);
                Some((s, e, true))
            }
        }
        KeyCode::Char('w') => {
            let mut t = cursor;
            for _ in 0..count {
                t = word_forward(text, t);
            }
            Some((cursor.min(t), cursor.max(t), false))
        }
        KeyCode::Char('e') => {
            let mut t = cursor;
            for _ in 0..count {
                t = word_end(text, t);
            }
            t = (t + 1).min(total);
            Some((cursor.min(t), cursor.max(t), false))
        }
        KeyCode::Char('b') => {
            let mut t = cursor;
            for _ in 0..count {
                t = word_backward(text, t);
            }
            Some((t.min(cursor), t.max(cursor), false))
        }
        KeyCode::Char('$') => Some((cursor, line_end_exclusive(text, cursor), false)),
        KeyCode::Char('0') => Some((line_start(text, cursor), cursor, false)),
        KeyCode::Char('G') => Some((line_start(text, cursor), total, true)),
        _ => None,
    }
}

fn perform_op(
    state: &mut VimState,
    op: char,
    start: usize,
    end: usize,
    linewise: bool,
) -> VimAction {
    match op {
        'd' => {
            let (t, removed) = delete_range(&state.text, start, end);
            state.text = t;
            state.register = removed;
            state.register_linewise = linewise;
            state.cursor = start;
            state.clamp_cursor();
            state.reset_pending();
            VimAction::Continue
        }
        'c' => {
            let (t, removed) = delete_range(&state.text, start, end);
            state.text = t;
            state.register = removed;
            state.register_linewise = linewise;
            state.cursor = start;
            state.clamp_cursor();
            state.mode = VimMode::Insert;
            state.reset_pending();
            VimAction::Continue
        }
        'y' => {
            let chars: Vec<char> = state.text.chars().collect();
            let s = start.min(chars.len());
            let e = end.min(chars.len()).max(s);
            state.register = chars[s..e].iter().collect();
            state.register_linewise = linewise;
            if !linewise {
                state.cursor = s;
            }
            state.reset_pending();
            VimAction::Continue
        }
        _ => {
            state.reset_pending();
            VimAction::Continue
        }
    }
}

/// Consume the motion key following a pending operator and perform the op.
pub(super) fn apply_operator(
    state: &mut VimState,
    op: char,
    k: KeyEvent,
    count: usize,
    _inner_w: u16,
    _prompt_w: u16,
) -> VimAction {
    // dgg / ccg / ygg: two-key sequence.
    if let KeyCode::Char('g') = k.code {
        if !state.pending_g {
            state.pending_g = true;
            return VimAction::Continue;
        }
        state.pending_g = false;
        let (s, e) = linewise_range_to_current(&state.text, state.cursor);
        return perform_op(state, op, s, e, true);
    }
    match operator_motion(&state.text, state.cursor, op, &k, count) {
        Some((s, e, linewise)) => perform_op(state, op, s, e, linewise),
        None => {
            state.status.clear();
            state.reset_pending();
            VimAction::Continue
        }
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

    fn norm(text: &str, cursor: usize) -> VimState {
        let mut s = VimState::new(text.to_string());
        s.mode = VimMode::Normal;
        s.cursor = cursor;
        s
    }

    #[test]
    fn delete_range_basic() {
        let (t, r) = delete_range("abcdef", 1, 3);
        assert_eq!(t, "adef");
        assert_eq!(r, "bc");
        // out-of-range start/end clamp to an empty range at EOF (no change)
        let (t, r) = delete_range("abc", 10, 20);
        assert_eq!(t, "abc");
        assert_eq!(r, "");
    }

    #[test]
    fn dd_deletes_line() {
        let mut s = norm("a\nb\nc", 2);
        apply_operator(&mut s, 'd', k('d'), 1, 80, 2);
        assert_eq!(s.text, "a\nc");
        assert_eq!(s.register, "b\n");
        assert!(s.register_linewise);
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn dd_count_two() {
        let mut s = norm("a\nb\nc\nd", 0);
        apply_operator(&mut s, 'd', k('d'), 2, 80, 2);
        assert_eq!(s.text, "c\nd");
    }

    #[test]
    fn dd_single_line_to_empty() {
        let mut s = norm("abc", 0);
        apply_operator(&mut s, 'd', k('d'), 1, 80, 2);
        assert_eq!(s.text, "");
        assert_eq!(s.register, "abc");
    }

    #[test]
    fn cc_keeps_empty_line_and_insert() {
        let mut s = norm("a\nbbb\nc", 2);
        apply_operator(&mut s, 'c', k('c'), 1, 80, 2);
        assert_eq!(s.text, "a\n\nc");
        assert_eq!(s.mode, VimMode::Insert);
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn yy_yanks_line_without_modifying() {
        let mut s = norm("a\nb\nc", 2);
        apply_operator(&mut s, 'y', k('y'), 1, 80, 2);
        assert_eq!(s.text, "a\nb\nc");
        assert_eq!(s.register, "b\n");
        assert!(s.register_linewise);
    }

    #[test]
    fn dw_deletes_word_and_leads_space() {
        let mut s = norm("foo bar baz", 0);
        apply_operator(&mut s, 'd', k('w'), 1, 80, 2);
        assert_eq!(s.text, "bar baz");
        assert_eq!(s.register, "foo ");
    }

    #[test]
    fn cw_changes_word() {
        let mut s = norm("foo bar", 0);
        apply_operator(&mut s, 'c', k('w'), 1, 80, 2);
        assert_eq!(s.text, "bar");
        assert_eq!(s.register, "foo ");
        assert_eq!(s.mode, VimMode::Insert);
    }

    #[test]
    fn yw_yanks_word() {
        let mut s = norm("foo bar", 0);
        apply_operator(&mut s, 'y', k('w'), 1, 80, 2);
        assert_eq!(s.register, "foo ");
        assert_eq!(s.text, "foo bar");
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn de_inclusive_and_d_dollar() {
        let mut s = norm("foo bar", 0);
        apply_operator(&mut s, 'd', k('e'), 1, 80, 2);
        assert_eq!(s.text, " bar");
        assert_eq!(s.register, "foo");
        let mut s = norm("foo bar", 0);
        apply_operator(&mut s, 'd', k('$'), 1, 80, 2);
        assert_eq!(s.text, "");
    }

    #[test]
    fn d0_and_db_backward() {
        let mut s = norm("foo bar", 4);
        apply_operator(&mut s, 'd', k('0'), 1, 80, 2);
        assert_eq!(s.text, "bar");
        assert_eq!(s.cursor, 0);
        let mut s = norm("foo bar baz", 8);
        apply_operator(&mut s, 'd', k('b'), 1, 80, 2);
        assert_eq!(s.text, "foo baz");
    }

    #[test]
    fn d_g_to_end_and_dgg() {
        let mut s = norm("a\nb\nc", 2);
        apply_operator(&mut s, 'd', k('G'), 1, 80, 2);
        assert_eq!(s.text, "a\n");
        // dgg: pending_g handling
        let mut s = norm("a\nb\nc", 4); // on "c"
        s.pending_op = Some('d');
        apply_operator(&mut s, 'd', k('g'), 1, 80, 2); // first g -> pending_g
        assert!(s.pending_g);
        apply_operator(&mut s, 'd', k('g'), 1, 80, 2); // second g -> commit
        assert_eq!(s.text, "");
    }

    #[test]
    fn unknown_motion_cancels_operator() {
        let mut s = norm("foo bar", 0);
        s.pending_op = Some('d');
        s.count = None;
        apply_operator(&mut s, 'd', k('z'), 1, 80, 2);
        assert_eq!(s.text, "foo bar"); // unchanged
        assert_eq!(s.pending_op, None); // cleared
    }
}
