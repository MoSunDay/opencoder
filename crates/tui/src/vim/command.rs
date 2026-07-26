//! Command-line mode (`:`) for the vim engine.
//!
//! Supports the subset of ex commands relevant to a single-buffer plan editor:
//! `:q`/`:q!` discard & exit, `:wq` save & exit. Any other command (including
//! `:w` and `:x`) is treated as unknown and returns to Normal mode.

use super::state::{VimAction, VimMode};
use crossterm::event::{KeyCode, KeyEvent};

pub fn handle_command(state: &mut super::state::VimState, k: KeyEvent) -> VimAction {
    match k.code {
        KeyCode::Esc => {
            state.mode = VimMode::Normal;
            state.cmdline.clear();
            state.reset_pending();
            VimAction::Continue
        }
        KeyCode::Backspace => {
            if state.cmdline.is_empty() {
                state.mode = VimMode::Normal;
                state.reset_pending();
            } else {
                let mut chars: Vec<char> = state.cmdline.chars().collect();
                chars.pop();
                state.cmdline = chars.into_iter().collect();
            }
            VimAction::Continue
        }
        KeyCode::Enter => {
            let cmd = state.cmdline.trim().to_string();
            state.cmdline.clear();
            state.reset_pending();
            match cmd.as_str() {
                "q" | "q!" => {
                    // discard edits and exit.
                    state.text = state.original.clone();
                    state.clamp_cursor();
                    VimAction::Exit
                }
                "wq" => {
                    // save (text already holds edits) and exit.
                    VimAction::Exit
                }
                "" => {
                    // empty command: just return to normal mode.
                    state.mode = VimMode::Normal;
                    VimAction::Continue
                }
                other => {
                    state.status = format!("Unknown command: :{}", other);
                    state.mode = VimMode::Normal;
                    VimAction::Continue
                }
            }
        }
        KeyCode::Char(c) if !c.is_control() => {
            state.cmdline.push(c);
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

    fn type_cmd(state: &mut VimState, cmd: &str) {
        state.mode = VimMode::Command;
        state.cmdline.clear();
        for ch in cmd.chars() {
            handle_command(state, k(ch));
        }
    }

    #[test]
    fn q_discards_and_exits() {
        let mut s = VimState::new("orig".to_string());
        s.text = "changed".to_string();
        type_cmd(&mut s, "q!");
        assert_eq!(handle_command(&mut s, enter()), VimAction::Exit);
        assert_eq!(s.text, "orig");
        assert!(!s.is_modified());
    }

    #[test]
    fn wq_saves_and_exit_keeping_text() {
        let mut s = VimState::new("orig".to_string());
        s.text = "changed".to_string();
        type_cmd(&mut s, "wq");
        assert_eq!(handle_command(&mut s, enter()), VimAction::Exit);
        assert_eq!(s.text, "changed");
        assert!(s.is_modified());
    }

    #[test]
    fn x_is_unknown_command() {
        let mut s = VimState::new("orig".to_string());
        s.text = "changed".to_string();
        type_cmd(&mut s, "x");
        assert_eq!(handle_command(&mut s, enter()), VimAction::Continue);
        assert_eq!(s.mode, VimMode::Normal);
        assert!(s.status.contains("Unknown command"));
        assert!(s.is_modified());
    }

    #[test]
    fn w_is_unknown_command() {
        let mut s = VimState::new("orig".to_string());
        s.text = "changed".to_string();
        type_cmd(&mut s, "w");
        assert_eq!(handle_command(&mut s, enter()), VimAction::Continue);
        assert_eq!(s.mode, VimMode::Normal);
        assert!(s.status.contains("Unknown command"));
        assert!(s.is_modified());
    }

    #[test]
    fn unknown_command_sets_status_and_returns_to_normal() {
        let mut s = VimState::new("abc".to_string());
        type_cmd(&mut s, "nope");
        assert_eq!(handle_command(&mut s, enter()), VimAction::Continue);
        assert_eq!(s.mode, VimMode::Normal);
        assert!(s.status.contains("Unknown command"));
        assert!(s.status.contains("nope"));
    }

    #[test]
    fn backspace_on_empty_cancels() {
        let mut s = VimState::new("abc".to_string());
        s.mode = VimMode::Command;
        s.cmdline.clear();
        assert_eq!(handle_command(&mut s, backspace()), VimAction::Continue);
        assert_eq!(s.mode, VimMode::Normal);
    }

    #[test]
    fn backspace_pops_char() {
        let mut s = VimState::new("abc".to_string());
        s.mode = VimMode::Command;
        s.cmdline = "wq".to_string();
        handle_command(&mut s, backspace());
        assert_eq!(s.cmdline, "w");
        assert_eq!(s.mode, VimMode::Command);
    }

    #[test]
    fn esc_cancels() {
        let mut s = VimState::new("abc".to_string());
        s.mode = VimMode::Command;
        s.cmdline = "wq".to_string();
        assert_eq!(handle_command(&mut s, esc()), VimAction::Continue);
        assert_eq!(s.mode, VimMode::Normal);
        assert_eq!(s.cmdline, "");
    }

    #[test]
    fn empty_enter_returns_to_normal() {
        let mut s = VimState::new("abc".to_string());
        s.mode = VimMode::Command;
        s.cmdline.clear();
        assert_eq!(handle_command(&mut s, enter()), VimAction::Continue);
        assert_eq!(s.mode, VimMode::Normal);
    }

    #[test]
    fn trailing_spaces_are_trimmed() {
        // ":q!   " should be treated as ":q!".
        let mut s = VimState::new("orig".to_string());
        s.text = "changed".to_string();
        type_cmd(&mut s, "q!   ");
        assert_eq!(handle_command(&mut s, enter()), VimAction::Exit);
        assert_eq!(s.text, "orig");
    }
}
