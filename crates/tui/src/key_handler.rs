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
    /// Steer submitted to a focused subagent's child session (not the parent).
    SubagentSteer(String),
    Queue(String),
    /// Tab-queue attempted while a running subagent is focused. A queue
    /// normally targets the *parent* session, which would leak input into
    /// the parent agent — so it is rejected here. The input box is left
    /// untouched so the user can press Enter to steer the subagent instead.
    QueueUnsupported,
    SwitchAgent(String),
    SwitchAgentNoClear(String),
    Cancel,
    /// Enter the plan-text editor (Shift+I in plan mode when idle).
    EnterPlanEdit,
    // Kept for the app.rs `KeyAction::SetSkill` plumbing (skill set/clear +
    // persistence). No longer constructed by the menu after the "clear skill"
    // row was removed, but the match arm in app.rs still handles it.
    #[allow(dead_code)]
    SetSkill(Option<(String, String)>),
    Clip,
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
    scroll: &mut u32,
    follow: &mut bool,
    last_esc: &mut Option<Instant>,
    skill_menu: &mut Option<SkillMenu>,
    inner_w: u16,
    prompt_w: u16,
    subagent_focused: bool,
    input_disabled: bool,
    undo_state: &mut crate::undo::UndoState,
    help_scroll: &mut u16,
    queue_scroll: &mut u32,
) -> KeyAction {
    // Modal skill picker: intercept all keys while open.
    if skill_menu.is_some() {
        return match handle_menu_key(skill_menu, k) {
            MenuOutcome::Quit => KeyAction::Quit,
            // A skill pick inserts a `$name` token at the cursor (the `$`
            // that opened the menu was already consumed). The skill body is
            // resolved and loaded on submit, not here, so picking is cheap and
            // reversible (backspace removes the token).
            MenuOutcome::Pick((name, _body)) => {
                let token = format!("${} ", name);
                let (s, i) = composer::insert_str(input, *cursor_idx, &token);
                *input = s;
                *cursor_idx = i;
                crate::undo::snapshot(undo_state, input, *cursor_idx, false);
                KeyAction::None
            }
            MenuOutcome::Idle => KeyAction::None,
        };
    }
    // Help popup scroll: when help is open, intercept scroll keys.
    if *show_help {
        match k.code {
            KeyCode::Up => {
                *help_scroll = help_scroll.saturating_sub(1);
                return KeyAction::None;
            }
            KeyCode::Down => {
                *help_scroll = help_scroll.saturating_add(1);
                return KeyAction::None;
            }
            KeyCode::PageUp => {
                *help_scroll = help_scroll.saturating_sub(10);
                return KeyAction::None;
            }
            KeyCode::PageDown => {
                *help_scroll = help_scroll.saturating_add(10);
                return KeyAction::None;
            }
            _ => {}
        }
    }
    // Queue/steer panel scroll keys: Shift+PageUp looks at older pending
    // entries, Shift+PageDown returns to the newest. Plain PageUp/PageDown
    // keep scrolling the body (below). A stale offset is clamped on the next
    // render, so these are safe even while the panel is hidden (plan mode).
    if k.modifiers.contains(KeyModifiers::SHIFT) {
        match k.code {
            KeyCode::PageUp => {
                *queue_scroll = queue_scroll.saturating_add(1);
                return KeyAction::None;
            }
            KeyCode::PageDown => {
                *queue_scroll = queue_scroll.saturating_sub(1);
                return KeyAction::None;
            }
            _ => {}
        }
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
                // Ctrl+C: interrupt the running task (same as double-Esc), when one
                // is active. When idle it is a no-op (use Ctrl+D to quit).
                KeyCode::Char('c') => {
                    return if running {
                        KeyAction::Cancel
                    } else {
                        KeyAction::None
                    };
                }
                KeyCode::Char('h') => {
                    *show_help = !*show_help;
                    return KeyAction::None;
                }
                _ => {}
            }
        }
        return KeyAction::None;
    }

    // Alt+Tab (and Shift+Tab) switches act <-> plan mode.
    if k.modifiers.contains(KeyModifiers::ALT) && matches!(k.code, KeyCode::Tab | KeyCode::BackTab)
    {
        let next = if agent == "plan" { "act" } else { "plan" };
        return KeyAction::SwitchAgent(next.into());
    }

    // Alt+F / Alt+B: readline-style forward/backward word movement.
    if k.modifiers.contains(KeyModifiers::ALT) {
        match k.code {
            KeyCode::Char('f') | KeyCode::Char('F') => {
                *cursor_idx = composer::forward_word(input, *cursor_idx);
                return KeyAction::None;
            }
            KeyCode::Char('b') | KeyCode::Char('B') => {
                *cursor_idx = composer::backward_word(input, *cursor_idx);
                return KeyAction::None;
            }
            _ => {}
        }
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
            KeyCode::Char('j') => {
                let (s, i) = composer::insert_newline(input, *cursor_idx);
                *input = s;
                *cursor_idx = i;
                crate::undo::snapshot(undo_state, input, *cursor_idx, false);
                return KeyAction::None;
            }
            // Ctrl+A / Ctrl+E: cursor to start / end of the input buffer.
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
                    crate::undo::snapshot(undo_state, input, *cursor_idx, false);
                }
                return KeyAction::None;
            }
            // Ctrl+U: clear the entire input line (readline unix-line-discard).
            // Undoable via snapshot so Ctrl+Z can restore, consistent with Ctrl+W.
            // Under kitty keyboard protocol crossterm may report the raw control
            // char `\u{15}` (NAK) with the CONTROL modifier set.
            KeyCode::Char('u') | KeyCode::Char('\u{15}') => {
                if !input.is_empty() {
                    input.clear();
                    *cursor_idx = 0;
                    crate::undo::snapshot(undo_state, input, *cursor_idx, false);
                }
                return KeyAction::None;
            }
            // Ctrl+T: switch act <-> plan mode WITHOUT clearing context
            // or auto-executing (pure mode toggle, same as Ctrl+Shift+Tab).
            // Preferred on terminals where Ctrl+Shift+Tab is captured by the
            // OS/shell before reaching the app. Input is left untouched.
            KeyCode::Char('t') => {
                let next = if agent == "plan" { "act" } else { "plan" };
                return KeyAction::SwitchAgentNoClear(next.into());
            }
            // Ctrl+V: paste image (screenshot bitmap) from the system
            // clipboard. Mirrors the legacy /clip command. Under kitty
            // keyboard protocol crossterm may report the raw control char
            // `\u{16}` (SYN) with the CONTROL modifier set.
            KeyCode::Char('v') | KeyCode::Char('\u{16}') => return KeyAction::Clip,
            // B4: Ctrl+C cancels the running task (equivalent to double-Esc).
            // Under raw mode Ctrl+C arrives as the ETX character (\u{3}),
            // not SIGINT, so it does not conflict with the supervisor's
            // signal handling. Also handled below via the raw-ETX fallback.
            KeyCode::Char('c') => {
                return if running {
                    KeyAction::Cancel
                } else {
                    KeyAction::None
                };
            }
            // Ctrl+Z: undo last edit.
            KeyCode::Char('z') => {
                if let Some((s, i)) = crate::undo::undo(undo_state, input, *cursor_idx) {
                    *input = s;
                    *cursor_idx = i;
                }
                return KeyAction::None;
            }
            // Ctrl+Y: redo last undone edit.
            KeyCode::Char('y') => {
                if let Some((s, i)) = crate::undo::redo(undo_state, input, *cursor_idx) {
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
                crate::undo::snapshot(undo_state, input, *cursor_idx, false);
                return KeyAction::None;
            }
            if input.trim().is_empty() {
                return KeyAction::None;
            }
            let text = input.trim().to_string();
            input.clear();
            *cursor_idx = 0;
            *hist_idx = None;
            crate::undo::reset(undo_state, input, *cursor_idx);
            // Enter = SubagentSteer when a running subagent is focused;
            // Steer when the parent is running; Submit when idle.
            if subagent_focused {
                KeyAction::SubagentSteer(text)
            } else if running {
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
            // Focused running subagent: a queue would be admitted to the parent
            // session and affect the parent agent — reject it instead, leaving
            // the typed text so Enter can submit it as a subagent steer.
            if subagent_focused {
                return KeyAction::QueueUnsupported;
            }
            let text = input.trim().to_string();
            input.clear();
            *cursor_idx = 0;
            *hist_idx = None;
            crate::undo::reset(undo_state, input, *cursor_idx);
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
                crate::undo::reset(undo_state, input, *cursor_idx);
                KeyAction::None
            }
        }
        KeyCode::Up => {
            let (row, _) = composer::cursor_row_col(input, *cursor_idx, inner_w, prompt_w);
            if row > 0 {
                *cursor_idx =
                    composer::move_cursor_vertical(input, *cursor_idx, -1, inner_w, prompt_w);
            } else {
                move_hist(history, hist_idx, input, cursor_idx, -1);
                crate::undo::reset(undo_state, input, *cursor_idx);
            }
            KeyAction::None
        }
        KeyCode::Down => {
            let total = composer::display_rows(input, inner_w, prompt_w) as usize;
            let (row, _) = composer::cursor_row_col(input, *cursor_idx, inner_w, prompt_w);
            if row + 1 < total {
                *cursor_idx =
                    composer::move_cursor_vertical(input, *cursor_idx, 1, inner_w, prompt_w);
            } else {
                move_hist(history, hist_idx, input, cursor_idx, 1);
                crate::undo::reset(undo_state, input, *cursor_idx);
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
        KeyCode::Backspace => {
            if let Some((s, i)) = composer::backspace(input, *cursor_idx) {
                *input = s;
                *cursor_idx = i;
                crate::undo::snapshot(undo_state, input, *cursor_idx, false);
            }
            KeyAction::None
        }
        KeyCode::Char(c) => {
            // Shift+I (uppercase I) enters plan-edit mode — but ONLY when in
            // plan mode, idle, and the input box is empty. Once the user starts
            // typing, regular `I` insertion resumes.
            if c == 'I' && agent == "plan" && !running && !input_disabled && input.is_empty() {
                return KeyAction::EnterPlanEdit;
            }
            // Fallback quit for terminals/crossterm configs that deliver Ctrl+D
            // (EOT, 0x04) as a raw control char without the CONTROL modifier
            // flag (the Ctrl-block match above would miss it).
            if c == '\u{4}' {
                return KeyAction::Quit;
            }
            // Raw ETX (Ctrl+C, 0x03) delivered without the CONTROL modifier
            // flag: interrupt the running task if one is active (mirrors the
            // Ctrl-block handling above); otherwise swallow it so it is not
            // inserted as a literal control char into the input buffer.
            if c == '\u{3}' {
                return if running {
                    KeyAction::Cancel
                } else {
                    KeyAction::None
                };
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
            crate::undo::snapshot(undo_state, input, *cursor_idx, true);
            KeyAction::None
        }
        _ => KeyAction::None,
    }
}

/// Handle body-scroll keys (PageUp / PageDown) uniformly.
/// Returns `true` when the key was consumed and scroll/follow updated.
pub(crate) fn apply_scroll(k: &KeyEvent, scroll: &mut u32, follow: &mut bool) -> bool {
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
#[path = "key_handler_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "key_handler_plan_edit_tests.rs"]
mod plan_edit_tests;

#[cfg(test)]
#[path = "key_handler_queue_scroll_tests.rs"]
mod queue_scroll_tests;
