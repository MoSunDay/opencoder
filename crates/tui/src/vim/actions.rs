//! Plain Normal-mode commands (step 6 of the dispatch): motions, jumps, insert
//! entries, `x`/`X`, `p`/`P`, `D`/`C`, mode switches, and search repeat.
//!
//! [`dispatch_plain`] assumes no operator is pending. Movement reuses
//! `crate::composer` for vertical wrap; delete/paste primitives live in
//! [`super::ops`] (shared with the operator path).

use super::motion::{
    line_count, line_end_exclusive, line_first_nonblank, line_start, line_start_by_number,
    word_backward, word_end, word_forward,
};
use super::ops::delete_range;
use super::search::find;
use super::state::{VimAction, VimMode, VimState};
use crate::composer;
use crossterm::event::{KeyCode, KeyEvent};

/// The plain-command dispatch (no operator pending). `count` is the resolved
/// repeat count; `total` is the current char count of the buffer.
pub(super) fn dispatch_plain(
    state: &mut VimState,
    k: KeyEvent,
    count: usize,
    total: usize,
    inner_w: u16,
    prompt_w: u16,
) -> VimAction {
    match k.code {
        KeyCode::Esc => {
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('h') => {
            state.cursor = state.cursor.saturating_sub(count);
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('l') => {
            state.cursor = (state.cursor + count).min(total);
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('j') => {
            for _ in 0..count {
                state.cursor =
                    composer::move_cursor_vertical(&state.text, state.cursor, 1, inner_w, prompt_w);
            }
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('k') => {
            for _ in 0..count {
                state.cursor = composer::move_cursor_vertical(
                    &state.text,
                    state.cursor,
                    -1,
                    inner_w,
                    prompt_w,
                );
            }
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('w') => {
            for _ in 0..count {
                state.cursor = word_forward(&state.text, state.cursor);
            }
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('b') => {
            for _ in 0..count {
                state.cursor = word_backward(&state.text, state.cursor);
            }
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('e') => {
            for _ in 0..count {
                state.cursor = word_end(&state.text, state.cursor);
            }
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('0') => {
            state.cursor = line_start(&state.text, state.cursor);
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('^') => {
            state.cursor = line_first_nonblank(&state.text, state.cursor);
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('$') => {
            let le = line_end_exclusive(&state.text, state.cursor);
            let ls = line_start(&state.text, state.cursor);
            state.cursor = le.saturating_sub(1).max(ls);
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('G') => {
            let target_line = if state.count.is_some() {
                count
            } else {
                line_count(&state.text)
            };
            let ls = line_start_by_number(&state.text, target_line);
            state.cursor = line_first_nonblank(&state.text, ls);
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('g') => {
            if state.pending_g {
                let target_line = state.count.unwrap_or(1);
                let ls = line_start_by_number(&state.text, target_line);
                state.cursor = line_first_nonblank(&state.text, ls);
                state.reset_pending();
            } else {
                state.pending_g = true;
            }
            VimAction::Continue
        }
        KeyCode::Char('i') => {
            state.mode = VimMode::Insert;
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('I') => {
            state.cursor = line_first_nonblank(&state.text, state.cursor);
            state.mode = VimMode::Insert;
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('a') => {
            state.cursor = (state.cursor + 1).min(total);
            state.mode = VimMode::Insert;
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('A') => {
            state.cursor = line_end_exclusive(&state.text, state.cursor);
            state.mode = VimMode::Insert;
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('o') => {
            let pos = line_end_exclusive(&state.text, state.cursor);
            let (t, i) = composer::insert_newline(&state.text, pos);
            state.text = t;
            state.cursor = i;
            state.mode = VimMode::Insert;
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('O') => {
            let pos = line_start(&state.text, state.cursor);
            let (t, _) = composer::insert_newline(&state.text, pos);
            state.text = t;
            state.cursor = pos; // before the inserted newline (new empty line above)
            state.mode = VimMode::Insert;
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('x') => {
            delete_chars_forward(state, count);
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('X') => {
            delete_chars_backward(state, count);
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('p') => {
            paste(state, true);
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('P') => {
            paste(state, false);
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('D') => {
            let le = line_end_exclusive(&state.text, state.cursor);
            let (t, removed) = delete_range(&state.text, state.cursor, le);
            state.text = t;
            state.register = removed;
            state.register_linewise = false;
            state.clamp_cursor();
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('C') => {
            let le = line_end_exclusive(&state.text, state.cursor);
            let (t, removed) = delete_range(&state.text, state.cursor, le);
            state.text = t;
            state.register = removed;
            state.register_linewise = false;
            state.cursor = state.cursor.min(state.char_count());
            state.mode = VimMode::Insert;
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('d') | KeyCode::Char('c') | KeyCode::Char('y') => {
            if let KeyCode::Char(op) = k.code {
                state.pending_op = Some(op);
            }
            VimAction::Continue
        }
        KeyCode::Char(':') => {
            state.mode = VimMode::Command;
            state.cmdline.clear();
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('/') => {
            state.mode = VimMode::Search;
            state.search_forward = true;
            state.search_input.clear();
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('?') => {
            state.mode = VimMode::Search;
            state.search_forward = false;
            state.search_input.clear();
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('n') => {
            repeat_search(state, true);
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Char('N') => {
            repeat_search(state, false);
            state.reset_pending();
            VimAction::Continue
        }
        _ => {
            state.reset_pending();
            VimAction::Continue
        }
    }
}

/// Delete `count` chars at/after the cursor (vim `x`).
fn delete_chars_forward(state: &mut VimState, count: usize) {
    let chars: Vec<char> = state.text.chars().collect();
    let len = chars.len();
    if state.cursor >= len {
        return;
    }
    let end = (state.cursor + count).min(len);
    let (t, removed) = delete_range(&state.text, state.cursor, end);
    state.text = t;
    state.register = removed;
    state.register_linewise = false;
    let new_len = len - (end - state.cursor);
    state.cursor = if new_len == 0 {
        0
    } else {
        state.cursor.min(new_len - 1)
    };
}

/// Delete `count` chars before the cursor (vim `X`).
fn delete_chars_backward(state: &mut VimState, count: usize) {
    let start = state.cursor.saturating_sub(count);
    if start == state.cursor {
        return;
    }
    let (t, removed) = delete_range(&state.text, start, state.cursor);
    state.text = t;
    state.register = removed;
    state.register_linewise = false;
    state.cursor = start;
}

/// Repeat the last committed search in the same (`n`) or opposite (`N`) dir.
fn repeat_search(state: &mut VimState, same_dir: bool) {
    if let Some((q, fwd)) = &state.last_search.clone() {
        let dir = if same_dir { *fwd } else { !*fwd };
        if let Some(pos) = find(&state.text, state.cursor, q, dir) {
            state.cursor = pos;
            state.status.clear();
        } else {
            state.status = format!("Pattern not found: {}", q);
        }
    }
}

/// Paste the register. `after` = `p` (after cursor), else `P` (before).
fn paste(state: &mut VimState, after: bool) {
    if state.register.is_empty() {
        return;
    }
    let total = state.char_count();
    if state.register_linewise {
        // normalize: strip leading newlines, ensure single trailing newline
        let body = state.register.trim_start_matches('\n');
        let body = if body.ends_with('\n') {
            body.to_string()
        } else {
            format!("{}\n", body)
        };
        if after {
            let cur_end = line_end_exclusive(&state.text, state.cursor);
            if cur_end < total {
                let pos = cur_end + 1;
                let (t, _) = composer::insert_str(&state.text, pos, &body);
                state.text = t;
                state.cursor = pos;
            } else {
                let pos = total;
                let ins = format!("\n{}", body);
                let (t, _) = composer::insert_str(&state.text, pos, &ins);
                state.text = t;
                state.cursor = pos + 1;
            }
        } else {
            let pos = line_start(&state.text, state.cursor);
            let (t, _) = composer::insert_str(&state.text, pos, &body);
            state.text = t;
            state.cursor = pos;
        }
    } else {
        let pos = if after {
            (state.cursor + 1).min(total)
        } else {
            state.cursor
        };
        let (t, _) = composer::insert_str(&state.text, pos, &state.register);
        state.text = t;
        let reg_chars = state.register.chars().count();
        state.cursor = if reg_chars == 0 {
            pos
        } else {
            pos + reg_chars - 1
        };
    }
}
