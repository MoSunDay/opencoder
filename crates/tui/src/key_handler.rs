//! Keyboard event handling — extracted from `app.rs` to keep file sizes
//! within the 800-line limit. Contains the `KeyAction` enum, the main
//! `handle_key` dispatcher, and the `move_hist` history-cycle helper.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::keymap::KeyBindings;

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
    /// Enter on a focused RUNNING subagent — steer the CHILD session, not the
    /// parent. The steer is admitted to the child session and pushed onto the
    /// child view's steer panel (see `subagent_input::admit_subagent_steer`);
    /// the parent's turn, skill tokens and steer panel are untouched.
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
    OpenKeymap,
    Clip,
    OpenCommand,
    Quit,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_key(
    k: KeyEvent,
    bindings: &KeyBindings,
    input: &mut String,
    cursor_idx: &mut usize,
    history: &[String],
    hist_idx: &mut Option<usize>,
    running: bool,
    agent: &str,
    scroll: &mut u32,
    follow: &mut bool,
    last_esc: &mut Option<Instant>,
    skill_menu: &mut Option<SkillMenu>,
    inner_w: u16,
    prompt_w: u16,
    subagent_focused: bool,
    input_disabled: bool,
    undo_state: &mut crate::undo::UndoState,
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
    // Queue/steer panel scroll keys: Shift+PageUp looks at older pending
    // entries (toward the top), Shift+PageDown moves toward newer ones
    // (toward the bottom). Plain PageUp/PageDown keep scrolling the body
    // (below). A stale offset is clamped on the next render, so these are
    // safe even while the panel is hidden (plan mode).
    if k.modifiers.contains(KeyModifiers::SHIFT) {
        match k.code {
            KeyCode::PageUp => {
                *queue_scroll = queue_scroll.saturating_sub(1);
                return KeyAction::None;
            }
            KeyCode::PageDown => {
                *queue_scroll = queue_scroll.saturating_add(1);
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
        if bindings.quit.matches(&k) {
            return KeyAction::Quit;
        }
        if bindings.cancel.matches(&k) {
            // Idle: quit like Ctrl+D. Running: cancel the in-flight turn.
            return if running {
                KeyAction::Cancel
            } else {
                KeyAction::Quit
            };
        }
        if bindings.help.matches(&k) {
            return KeyAction::OpenKeymap;
        }
        return KeyAction::None;
    }

    // switch_mode_clear (default: Alt+Tab): switches act <-> plan mode.
    if bindings.switch_mode_clear.matches(&k) {
        let next = if agent == "plan" { "act" } else { "plan" };
        return KeyAction::SwitchAgent(next.into());
    }

    // forward_word / backward_word (default: Alt+F / Alt+B).
    if bindings.forward_word.matches(&k) {
        *cursor_idx = composer::forward_word(input, *cursor_idx);
        return KeyAction::None;
    }
    if bindings.backward_word.matches(&k) {
        *cursor_idx = composer::backward_word(input, *cursor_idx);
        return KeyAction::None;
    }

    // switch_mode_keep (default: Ctrl+Shift+Tab): toggle act <-> plan mode
    // WITHOUT clearing context. The `matches` method normalizes BackTab ≡
    // Tab+SHIFT so both terminal variants are covered.
    if bindings.switch_mode_keep.matches(&k) {
        let next = if agent == "plan" { "act" } else { "plan" };
        return KeyAction::SwitchAgentNoClear(next.into());
    }

    // --- Config-driven Ctrl bindings ---
    if bindings.quit.matches(&k) {
        return KeyAction::Quit;
    }
    if bindings.help.matches(&k) {
        return KeyAction::OpenKeymap;
    }
    if bindings.newline.matches(&k) {
        let (s, i) = composer::insert_newline(input, *cursor_idx);
        *input = s;
        *cursor_idx = i;
        crate::undo::snapshot(undo_state, input, *cursor_idx, false);
        return KeyAction::None;
    }
    if bindings.cursor_home.matches(&k) {
        *cursor_idx = 0;
        return KeyAction::None;
    }
    if bindings.cursor_end.matches(&k) {
        *cursor_idx = input.chars().count();
        return KeyAction::None;
    }
    if bindings.delete_word.matches(&k) {
        if let Some((s, i)) = composer::delete_word_back(input, *cursor_idx) {
            *input = s;
            *cursor_idx = i;
            crate::undo::snapshot(undo_state, input, *cursor_idx, false);
        }
        return KeyAction::None;
    }
    if bindings.clear_input.matches(&k) {
        if !input.is_empty() {
            input.clear();
            *cursor_idx = 0;
            crate::undo::snapshot(undo_state, input, *cursor_idx, false);
        }
        return KeyAction::None;
    }
    if bindings.switch_mode.matches(&k) {
        let next = if agent == "plan" { "act" } else { "plan" };
        return KeyAction::SwitchAgentNoClear(next.into());
    }
    if bindings.paste_image.matches(&k) {
        return KeyAction::Clip;
    }
    if bindings.cancel.matches(&k) {
        // Idle: quit like Ctrl+D. Running: cancel the in-flight turn.
        return if running {
            KeyAction::Cancel
        } else {
            KeyAction::Quit
        };
    }
    if bindings.undo.matches(&k) {
        if let Some((s, i)) = crate::undo::undo(undo_state, input, *cursor_idx) {
            *input = s;
            *cursor_idx = i;
        }
        return KeyAction::None;
    }
    if bindings.redo.matches(&k) {
        if let Some((s, i)) = crate::undo::redo(undo_state, input, *cursor_idx) {
            *input = s;
            *cursor_idx = i;
        }
        return KeyAction::None;
    }
    // Swallow any remaining Ctrl+key that didn't match a binding.
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        return KeyAction::None;
    }
    match k.code {
        KeyCode::BackTab => {
            // Shift+Tab = primary mode switch (codex-cli style), BUT a
            // compound `/plan <content>` input is submitted as a plan-mode
            // prompt rather than just toggling the agent (mirrors the
            // Enter/Tab submit + buffer-clear flow).
            if let Some(text) = crate::control_helpers::plan_compound_for_submit(input) {
                input.clear();
                *cursor_idx = 0;
                *hist_idx = None;
                crate::undo::reset(undo_state, input, *cursor_idx);
                return KeyAction::Submit(text);
            }
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
            // Enter = steer the focused CHILD session when a running subagent
            // is focused; steer the parent when it is running; Submit when idle.
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
            // Double-Esc within the window while running => hard-abort.
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
            // Alt+Char: tmux escape-time merges Esc into Alt+char, so unhandled
            // Alt combos must never reach the input box (ghost garbage guard).
            // Explicit Alt bindings (f/F/b/B/Tab) are handled above; Alt+Ctrl
            // combos keep their raw semantics.
            if k.modifiers.contains(KeyModifiers::ALT)
                && !k.modifiers.contains(KeyModifiers::CONTROL)
            {
                return KeyAction::None;
            }
            // Shift+I (uppercase I) enters plan-mode edit — ONLY when in
            // plan mode, idle, and the input box is empty. Once the user starts
            // typing, regular `I` insertion resumes.
            if c == 'I' && agent == "plan" && !running && !input_disabled && input.is_empty() {
                return KeyAction::EnterPlanEdit;
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
