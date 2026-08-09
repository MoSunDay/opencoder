//! Copy/selection mode: a cross-terminal-reliable way to select and copy text
//! from the information area.
//!
//! Entering copy mode (default `Ctrl+G`, configurable) suspends the TUI's own
//! mouse capture AND disables tmux's `mouse` interception, handing raw mouse
//! drags back to the terminal emulator so its native text selection works.
//! Every terminal -- not just Kitty/WezTerm -- can then select/copy using its
//! own shortcuts.
//!
//! On exit the TUI mouse capture is resumed, but tmux's `mouse` is left off
//! (see [`crate::tmux_mouse`]) to avoid re-introducing the selection fight.

use crossterm::event::{KeyCode, KeyEvent};

use crate::keymap::KeyBindings;
use crate::terminal::{resume_mouse_capture, suspend_mouse_capture};

/// Enter copy/selection mode: suspend our mouse capture and turn off tmux
/// mouse interception. Best-effort -- terminal errors are ignored.
pub fn enter() {
    let _ = suspend_mouse_capture();
    // Discard previous state: we keep tmux mouse off on exit.
    let _ = crate::tmux_mouse::disable();
}

/// Exit copy/selection mode: resume our mouse capture. tmux mouse is left off.
pub fn exit() {
    let _ = resume_mouse_capture();
}

/// Whether copy/selection mode is currently suppressing mouse interactions.
/// True when the explicit toggle is on OR the user is holding Shift (the
/// Kitty-keyboard-protocol native-selection path tracked by `terminal.rs`).
pub fn is_active(copy_mode: bool, shift_held: bool) -> bool {
    copy_mode || shift_held
}

/// Pure decision logic for a key in the copy-mode context. No I/O -- fully
/// unit-testable. Returns `(new_active, consumed)`:
/// - toggle key pressed -> flip `active`, consumed.
/// - active + any other key -> swallowed (consumed); `Esc` clears `active`.
/// - inactive + non-toggle key -> passed through (not consumed).
fn next_state(k: &KeyEvent, active: bool, keymap: &KeyBindings) -> (bool, bool) {
    if keymap.copy_mode.matches(k) {
        (!active, true)
    } else if active {
        let exiting = k.code == KeyCode::Esc;
        (active && !exiting, true)
    } else {
        (active, false)
    }
}

/// Handle a key for copy-mode toggle / input swallowing, performing the
/// enter/exit side effects. Returns `true` if the key was consumed (the caller
/// should mark the frame dirty and `continue`).
pub(crate) fn handle_key(k: &KeyEvent, copy_mode: &mut bool, keymap: &KeyBindings) -> bool {
    let prev = *copy_mode;
    let (next, consumed) = next_state(k, prev, keymap);
    if consumed {
        *copy_mode = next;
        if next && !prev {
            enter();
        } else if !next && prev {
            exit();
        }
    }
    consumed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use opencoder_core::Config;

    fn keybindings() -> KeyBindings {
        KeyBindings::from_config(&Config::default())
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn plain(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn is_active_truth_table() {
        assert!(!is_active(false, false));
        assert!(is_active(true, false));
        assert!(is_active(false, true));
        assert!(is_active(true, true));
    }

    #[test]
    fn toggle_key_flips_state() {
        let kb = keybindings();
        // Default copy-mode key is Ctrl+G.
        assert_eq!(next_state(&ctrl('g'), false, &kb), (true, true));
        assert_eq!(next_state(&ctrl('g'), true, &kb), (false, true));
    }

    #[test]
    fn active_mode_swallows_other_keys() {
        let kb = keybindings();
        // A plain letter is swallowed while copy mode is active.
        assert_eq!(next_state(&plain('x'), true, &kb), (true, true));
        assert_eq!(next_state(&plain('a'), true, &kb), (true, true));
    }

    #[test]
    fn esc_exits_active_mode() {
        let kb = keybindings();
        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(next_state(&esc, true, &kb), (false, true));
    }

    #[test]
    fn inactive_passes_through_non_toggle_keys() {
        let kb = keybindings();
        assert_eq!(next_state(&plain('x'), false, &kb), (false, false));
        assert_eq!(
            next_state(&ctrl('g'), false, &kb),
            (true, true),
            "toggle must still fire when inactive"
        );
    }
}
