//! Keyboard event handling — extracted from `app.rs` to keep file sizes
//! within the 800-line limit. Contains the `KeyAction` enum, the main
//! `handle_key` dispatcher, and the `move_hist` history-cycle helper.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use opencoder_core::discover_skills;

use crate::composer;
use crate::menu::{handle_menu_key, MenuOutcome, SkillMenu};

/// Window for double-Esc hard-abort (milliseconds).
pub(crate) const ESC_CANCEL_WINDOW_MS: u64 = 350;

/// Decision returned by `handle_key` for the event loop to act on.
#[derive(Debug)]
pub(crate) enum KeyAction {
    None,
    Submit(String),
    Steer(String),
    Queue(String),
    SwitchAgent(String),
    SwitchAgentNoClear(String),
    Cancel,
    // Kept for the app.rs `KeyAction::SetSkill` plumbing (skill set/clear +
    // persistence). No longer constructed by the menu after the "clear skill"
    // row was removed, but the match arm in app.rs still handles it.
    #[allow(dead_code)]
    SetSkill(Option<(String, String)>),
    OpenCommand,
    Quit,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_key(
    k: KeyEvent,
    input: &mut String,
    cursor_idx: &mut usize,
    history: &[String],
    hist_idx: &mut Option<usize>,
    running: bool,
    agent: &str,
    show_help: &mut bool,
    scroll: &mut u16,
    follow: &mut bool,
    last_esc: &mut Option<Instant>,
    skill_menu: &mut Option<SkillMenu>,
    inner_w: u16,
    prompt_w: u16,
    input_disabled: bool,
) -> KeyAction {
    // Modal skill picker: intercept all keys while open.
    if skill_menu.is_some() {
        return match handle_menu_key(skill_menu, k) {
            MenuOutcome::Quit => KeyAction::Quit,
            // A skill pick inserts a `{$name}` token at the cursor (the `$`
            // that opened the menu was already consumed). The skill body is
            // resolved and loaded on submit, not here, so picking is cheap and
            // reversible (backspace removes the token).
            MenuOutcome::Pick((name, _body)) => {
                let token = format!("{{${}}}", name);
                let (s, i) = composer::insert_str(input, *cursor_idx, &token);
                *input = s;
                *cursor_idx = i;
                KeyAction::None
            }
            MenuOutcome::Idle => KeyAction::None,
        };
    }
    // Alt+Tab (and Shift+Tab) switches act <-> plan mode.
    if k.modifiers.contains(KeyModifiers::ALT) && matches!(k.code, KeyCode::Tab | KeyCode::BackTab)
    {
        let next = if agent == "plan" { "act" } else { "plan" };
        return KeyAction::SwitchAgent(next.into());
    }

    // Ctrl+Shift+Tab: switch act <-> plan mode WITHOUT clearing context or
    // auto-executing (pure mode toggle, keeps the full transcript). Must be
    // checked before the CONTROL branch which would otherwise swallow
    // Tab/BackTab. Terminals report this as BackTab+CONTROL, or (under kitty
    // keyboard protocol with full disambiguation) Tab+CONTROL+SHIFT.
    if k.modifiers.contains(KeyModifiers::CONTROL)
        && (matches!(k.code, KeyCode::BackTab)
            || (k.modifiers.contains(KeyModifiers::SHIFT) && matches!(k.code, KeyCode::Tab)))
    {
        let next = if agent == "plan" { "act" } else { "plan" };
        return KeyAction::SwitchAgentNoClear(next.into());
    }

    // Body scroll keys (PageUp / PageDown) — shared between enabled
    // and disabled (subagent-focus) states so scrolling always works.
    if apply_scroll(&k, scroll, follow) {
        return KeyAction::None;
    }

    // Subagent-focus view: disable text input, submit, steer, queue. Only
    // scroll (handled above) and global keys (Quit, Help) are honoured.
    if input_disabled {
        if k.modifiers.contains(KeyModifiers::CONTROL) {
            match k.code {
                KeyCode::Char('d') | KeyCode::Char('\u{4}') => return KeyAction::Quit,
                KeyCode::Char('h') => {
                    *show_help = !*show_help;
                    return KeyAction::None;
                }
                _ => {}
            }
        }
        return KeyAction::None;
    }

    if k.modifiers.contains(KeyModifiers::CONTROL) {
        match k.code {
            // Ctrl+D quits. Under Kitty keyboard protocol
            // (DISAMBIGUATE_ESCAPE_CODES) crossterm reports this as the raw
            // control char `\u{4}` (EOT) with the CONTROL modifier set.
            KeyCode::Char('d') | KeyCode::Char('\u{4}') => return KeyAction::Quit,
            KeyCode::Char('h') => {
                *show_help = !*show_help;
                return KeyAction::None;
            }
            KeyCode::Char('n') => {
                move_hist(history, hist_idx, input, cursor_idx, 1);
                return KeyAction::None;
            }
            KeyCode::Char('p') => {
                move_hist(history, hist_idx, input, cursor_idx, -1);
                return KeyAction::None;
            }
            KeyCode::Char('j') => {
                let (s, i) = composer::insert_newline(input, *cursor_idx);
                *input = s;
                *cursor_idx = i;
                return KeyAction::None;
            }
            // Ctrl+A / Ctrl+E: cursor to start / end of the input buffer
            // (same as Home / End).
            KeyCode::Char('a') => {
                *cursor_idx = 0;
                return KeyAction::None;
            }
            KeyCode::Char('e') => {
                *cursor_idx = input.chars().count();
                return KeyAction::None;
            }
            // Ctrl+W: delete the word before the cursor (readline
            // backward-kill-word / unix-word-rubout, same as terminal).
            KeyCode::Char('w') => {
                if let Some((s, i)) = composer::delete_word_back(input, *cursor_idx) {
                    *input = s;
                    *cursor_idx = i;
                }
                return KeyAction::None;
            }
            _ => return KeyAction::None,
        }
    }
    match k.code {
        KeyCode::BackTab => {
            // Shift+Tab = primary mode switch (codex-cli style).
            let next = if agent == "plan" { "act" } else { "plan" };
            KeyAction::SwitchAgent(next.into())
        }
        KeyCode::Enter => {
            // Shift+Enter / Alt+Enter insert a newline (multi-line input).
            if k.modifiers
                .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT)
            {
                let (s, i) = composer::insert_newline(input, *cursor_idx);
                *input = s;
                *cursor_idx = i;
                return KeyAction::None;
            }
            if input.trim().is_empty() {
                return KeyAction::None;
            }
            let text = input.trim().to_string();
            input.clear();
            *cursor_idx = 0;
            *hist_idx = None;
            // Enter = Steer when running (strong intervention, promoted at
            // turn boundary); normal submit when idle.
            if running {
                KeyAction::Steer(text)
            } else {
                KeyAction::Submit(text)
            }
        }
        KeyCode::Tab => {
            // Tab = follow-up (queue) when running; normal submit when idle.
            if input.trim().is_empty() {
                return KeyAction::None;
            }
            let text = input.trim().to_string();
            input.clear();
            *cursor_idx = 0;
            *hist_idx = None;
            if running {
                KeyAction::Queue(text)
            } else {
                KeyAction::Submit(text)
            }
        }
        KeyCode::Esc => {
            // 1) If help is open, Esc just closes it.
            if *show_help {
                *show_help = false;
                return KeyAction::None;
            }
            // 2) Double-Esc within the window while running => hard-abort.
            let now = Instant::now();
            let is_double = running
                && last_esc
                    .map(|t| now.duration_since(t) < Duration::from_millis(ESC_CANCEL_WINDOW_MS))
                    .unwrap_or(false);
            if is_double {
                *last_esc = None;
                KeyAction::Cancel
            } else {
                *last_esc = Some(now);
                input.clear();
                *cursor_idx = 0;
                *hist_idx = None;
                KeyAction::None
            }
        }
        KeyCode::Up => {
            if input.contains('\n') {
                *cursor_idx =
                    composer::move_cursor_vertical(input, *cursor_idx, -1, inner_w, prompt_w);
            } else {
                move_hist(history, hist_idx, input, cursor_idx, -1);
            }
            KeyAction::None
        }
        KeyCode::Down => {
            if input.contains('\n') {
                *cursor_idx =
                    composer::move_cursor_vertical(input, *cursor_idx, 1, inner_w, prompt_w);
            } else {
                move_hist(history, hist_idx, input, cursor_idx, 1);
            }
            KeyAction::None
        }
        KeyCode::Left => {
            *cursor_idx = cursor_idx.saturating_sub(1);
            KeyAction::None
        }
        KeyCode::Right => {
            *cursor_idx = (*cursor_idx + 1).min(input.chars().count());
            KeyAction::None
        }
        KeyCode::Home => {
            *cursor_idx = 0;
            KeyAction::None
        }
        KeyCode::End => {
            *cursor_idx = input.chars().count();
            KeyAction::None
        }
        KeyCode::Backspace => {
            if let Some((s, i)) = composer::backspace(input, *cursor_idx) {
                *input = s;
                *cursor_idx = i;
            }
            KeyAction::None
        }
        KeyCode::Char(c) => {
            // Fallback quit for terminals/crossterm configs that deliver Ctrl+D
            // (EOT, 0x04) as a raw control char without the CONTROL modifier
            // flag (the Ctrl-block match above would miss it).
            if c == '\u{4}' {
                return KeyAction::Quit;
            }
            // Swallow raw ETX (Ctrl+C, 0x03) so it is not inserted as a literal
            // control char into the input buffer.
            if c == '\u{3}' {
                return KeyAction::None;
            }
            if c == '$' {
                *skill_menu = Some(SkillMenu::new(discover_skills()));
                return KeyAction::None;
            }
            // `/` on empty input opens the slash-command picker. Bare `/` +
            // Enter defaults to /task (first row) for muscle memory.
            if c == '/' && input.is_empty() && *cursor_idx == 0 {
                return KeyAction::OpenCommand;
            }
            let (s, i) = composer::insert_char(input, *cursor_idx, c);
            *input = s;
            *cursor_idx = i;
            KeyAction::None
        }
        _ => KeyAction::None,
    }
}

/// Handle body-scroll keys (PageUp / PageDown) uniformly.
/// Returns `true` when the key was consumed and scroll/follow updated.
pub(crate) fn apply_scroll(k: &KeyEvent, scroll: &mut u16, follow: &mut bool) -> bool {
    match k.code {
        KeyCode::PageUp => {
            *scroll = scroll.saturating_sub(20);
            *follow = false;
            true
        }
        KeyCode::PageDown => {
            *follow = true;
            true
        }
        _ => false,
    }
}

fn move_hist(
    history: &[String],
    hist_idx: &mut Option<usize>,
    input: &mut String,
    cursor_idx: &mut usize,
    delta: i32,
) {
    if history.is_empty() {
        return;
    }
    // If not currently browsing history, Down is a no-op (don't wipe input).
    if delta > 0 && hist_idx.is_none() {
        return;
    }
    let cur = hist_idx.unwrap_or(history.len());
    let next = (cur as i32 + delta).clamp(0, history.len() as i32) as usize;
    if next < history.len() {
        *hist_idx = Some(next);
        *input = history[next].clone();
    } else {
        *hist_idx = None;
        input.clear();
    }
    *cursor_idx = input.chars().count();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_scroll_page_up() {
        let mut scroll = 50u16;
        let mut follow = true;
        let k = KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE);
        assert!(apply_scroll(&k, &mut scroll, &mut follow));
        assert_eq!(scroll, 30);
        assert!(!follow);
    }

    #[test]
    fn apply_scroll_page_down() {
        let mut scroll = 50u16;
        let mut follow = false;
        let k = KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE);
        assert!(apply_scroll(&k, &mut scroll, &mut follow));
        assert!(follow);
    }

    #[test]
    fn apply_scroll_char_not_consumed() {
        let mut scroll = 50u16;
        let mut follow = true;
        let k = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        assert!(!apply_scroll(&k, &mut scroll, &mut follow));
        assert_eq!(scroll, 50);
        assert!(follow);
    }

    #[test]
    fn handle_key_disabled_blocks_char() {
        let mut input = String::new();
        let mut cursor = 0usize;
        let history: Vec<String> = Vec::new();
        let mut hist_idx: Option<usize> = None;
        let mut show_help = false;
        let mut scroll = 0u16;
        let mut follow = true;
        let mut last_esc: Option<Instant> = None;
        let mut skill_menu: Option<SkillMenu> = None;

        let action = handle_key(
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
            &mut input, &mut cursor, &history, &mut hist_idx, false, "act",
            &mut show_help, &mut scroll, &mut follow, &mut last_esc,
            &mut skill_menu, 80, 2, true,
        );
        assert!(matches!(action, KeyAction::None));
        assert!(input.is_empty());
    }

    #[test]
    fn handle_key_disabled_blocks_enter() {
        let mut input = String::new();
        let mut cursor = 0usize;
        let history: Vec<String> = Vec::new();
        let mut hist_idx: Option<usize> = None;
        let mut show_help = false;
        let mut scroll = 0u16;
        let mut follow = true;
        let mut last_esc: Option<Instant> = None;
        let mut skill_menu: Option<SkillMenu> = None;

        let action = handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut input, &mut cursor, &history, &mut hist_idx, false, "act",
            &mut show_help, &mut scroll, &mut follow, &mut last_esc,
            &mut skill_menu, 80, 2, true,
        );
        assert!(matches!(action, KeyAction::None));
    }

    #[test]
    fn handle_key_disabled_allows_scroll() {
        let mut input = String::new();
        let mut cursor = 0usize;
        let history: Vec<String> = Vec::new();
        let mut hist_idx: Option<usize> = None;
        let mut show_help = false;
        let mut scroll = 50u16;
        let mut follow = true;
        let mut last_esc: Option<Instant> = None;
        let mut skill_menu: Option<SkillMenu> = None;

        let action = handle_key(
            KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
            &mut input, &mut cursor, &history, &mut hist_idx, false, "act",
            &mut show_help, &mut scroll, &mut follow, &mut last_esc,
            &mut skill_menu, 80, 2, true,
        );
        assert!(matches!(action, KeyAction::None));
        assert_eq!(scroll, 30);
        assert!(!follow);
    }

    #[test]
    fn handle_key_disabled_allows_quit() {
        let mut input = String::new();
        let mut cursor = 0usize;
        let history: Vec<String> = Vec::new();
        let mut hist_idx: Option<usize> = None;
        let mut show_help = false;
        let mut scroll = 0u16;
        let mut follow = true;
        let mut last_esc: Option<Instant> = None;
        let mut skill_menu: Option<SkillMenu> = None;

        let action = handle_key(
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
            &mut input, &mut cursor, &history, &mut hist_idx, false, "act",
            &mut show_help, &mut scroll, &mut follow, &mut last_esc,
            &mut skill_menu, 80, 2, true,
        );
        assert!(matches!(action, KeyAction::Quit));
    }

    #[test]
    fn move_hist_down_does_not_clear_input_when_not_browsing() {
        let history = vec!["previous command".to_string()];
        let mut hist_idx = None;
        let mut input = "typing something".to_string();
        let mut cursor = 5;
        move_hist(&history, &mut hist_idx, &mut input, &mut cursor, 1);
        assert_eq!(
            input, "typing something",
            "Down should not clear input when not browsing history"
        );
        assert_eq!(hist_idx, None, "hist_idx should remain None");
    }

    #[test]
    fn move_hist_up_loads_previous_entry() {
        let history = vec!["cmd1".to_string(), "cmd2".to_string()];
        let mut hist_idx = None;
        let mut input = "current".to_string();
        let mut cursor = 0;
        move_hist(&history, &mut hist_idx, &mut input, &mut cursor, -1);
        assert_eq!(
            input, "cmd2",
            "Up should load the most recent history entry"
        );
        assert_eq!(hist_idx, Some(1));
    }

    #[test]
    fn move_hist_down_after_up_restores_blank() {
        let history = vec!["cmd1".to_string()];
        let mut hist_idx = None;
        let mut input = "original".to_string();
        let mut cursor = 0;
        // Up loads history
        move_hist(&history, &mut hist_idx, &mut input, &mut cursor, -1);
        assert_eq!(input, "cmd1");
        // Down goes past the end → clears
        move_hist(&history, &mut hist_idx, &mut input, &mut cursor, 1);
        assert_eq!(input, "", "Down past newest should clear input");
        assert_eq!(hist_idx, None);
    }
}
