//! Keyboard event handling — extracted from `app.rs` to keep file sizes
//! within the 800-line limit. Contains the `KeyAction` enum, the main
//! `handle_key` dispatcher, and the `move_hist` history-cycle helper.

use std::path::Path;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::keymap::KeyBindings;

use opencoder_core::discover_skills;

use crate::composer;
use crate::file_menu::{handle_file_key, FileMenu, FileOutcome};
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
    /// A textual mode command was submitted while work is running. The input
    /// remains untouched so the user can retry at an idle boundary.
    ModeSwitchBlocked,
    SwitchAgent(String),
    SwitchAgentNoClear(String),
    Cancel,
    /// Enter the plan-text editor (Shift+I in plan mode when idle).
    EnterPlanEdit,
    /// Activate a skill picked from the `$` menu, or clear the active skill
    /// (None) via the menu's dedicated clear row. app.rs routes both through
    /// `apply_skill_selection`, which persists set (skill=…) and clear
    /// (clear_skill) to the store.
    SetSkill(Option<(String, String)>),
    OpenKeymap,
    Clip,
    OpenCommand,
    /// Execute a local `!cmd` — run a non-interactive shell command.
    Bash(String),
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
    file_menu: &mut Option<FileMenu>,
    workdir: &Path,
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
            // The dedicated clear row: deactivate the active skill instead
            // of picking one. Routed through the same `SetSkill(None)`
            // plumbing as a pick, so app.rs persists the clear.
            MenuOutcome::Clear => KeyAction::SetSkill(None),
            MenuOutcome::Idle => KeyAction::None,
        };
    }
    // File-mention picker (`@`): intercept all keys while open, mirroring
    // the `$` skill picker above. A pick inserts the `@relative/path `
    // token at the cursor — the trigger `@` was consumed on open, so the
    // pick re-emits it; the marker keeps the token expandable to an
    // absolute path at submit time (mention_resolve).
    if file_menu.is_some() {
        return match handle_file_key(file_menu, k) {
            FileOutcome::Pick(token) => {
                let (s, i) = composer::insert_str(input, *cursor_idx, &token);
                *input = s;
                *cursor_idx = i;
                crate::undo::snapshot(undo_state, input, *cursor_idx, false);
                KeyAction::None
            }
            FileOutcome::Close | FileOutcome::Idle => KeyAction::None,
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
    // scroll (handled above), mode-switch keys (below), and global keys
    // (Quit, Help) are honoured.
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
        // Mode-switch keys stay live in the subagent-focus (input-disabled)
        // view: leaving/switching mode must never be blocked by view state.
        // All four funnel into handle_switch_agent, whose running gate
        // blocks BOTH directions while a turn/subagent is live (busy hint).
        // The `/plan <content>` compound-submit branch of
        // the enabled path is intentionally skipped — input is disabled here.
        // A plain BackTab carries no CTRL/ALT modifier, so it cannot
        // mis-match the switch_mode_clear / switch_mode_keep bindings above
        // (keymap `matches` requires exact modifiers besides lenient SHIFT).
        if bindings.switch_mode_clear.matches(&k) {
            let next = if agent == "plan" { "act" } else { "plan" };
            return KeyAction::SwitchAgent(next.into());
        }
        if bindings.switch_mode_keep.matches(&k) {
            let next = if agent == "plan" { "act" } else { "plan" };
            return KeyAction::SwitchAgentNoClear(next.into());
        }
        // switch_mode (default ctrl+t): same NoClear semantics as the
        // enabled path — a customized chord must not go dead just because
        // a subagent is focused.
        if bindings.switch_mode.matches(&k) {
            let next = if agent == "plan" { "act" } else { "plan" };
            return KeyAction::SwitchAgentNoClear(next.into());
        }
        if k.code == KeyCode::BackTab {
            let next = if agent == "plan" { "act" } else { "plan" };
            return KeyAction::SwitchAgent(next.into());
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
                if running {
                    return KeyAction::ModeSwitchBlocked;
                }
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
            if running && opencoder_session::control_cmd::is_mode_control(&text) {
                return KeyAction::ModeSwitchBlocked;
            }
            input.clear();
            *cursor_idx = 0;
            *hist_idx = None;
            crate::undo::reset(undo_state, input, *cursor_idx);
            // `!cmd` prefix → local non-interactive command execution.
            if let Some(cmd) = text.strip_prefix('!') {
                let cmd = cmd.trim();
                if !cmd.is_empty() {
                    return KeyAction::Bash(cmd.to_string());
                }
                return KeyAction::None;
            }
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
            let text = input.trim().to_string();
            if running && opencoder_session::control_cmd::is_mode_control(&text) {
                return KeyAction::ModeSwitchBlocked;
            }
            // Focused running subagent: a queue would be admitted to the parent
            // session and affect the parent agent — reject it instead, leaving
            // the typed text so Enter can submit it as a subagent steer.
            if subagent_focused {
                return KeyAction::QueueUnsupported;
            }
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
            // `@` at a token start opens the file-mention picker; the
            // character itself is consumed — the pick later re-emits it as
            // part of the `@relative/path ` token. A mid-token `@` (emails
            // like a@b.com) never triggers.
            if char_opens_file_menu(input, *cursor_idx, c) {
                *file_menu = Some(FileMenu::new(workdir));
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

/// Whether typing `c` at char-index `cursor_idx` in `input` should open
/// the file-mention picker: `@` at a token start (start of input or right
/// after whitespace). Mid-token `@` (emails like `a@b.com`) never
/// triggers. Public so the file-mention e2e (`tests/file_mention_flow.rs`)
/// drives the production trigger predicate instead of re-implementing it.
pub fn char_opens_file_menu(input: &str, cursor_idx: usize, c: char) -> bool {
    c == '@'
        && (cursor_idx == 0
            || input
                .chars()
                .nth(cursor_idx - 1)
                .is_some_and(char::is_whitespace))
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
#[path = "key_handler_disabled_mode_tests.rs"]
mod disabled_mode_tests;

#[cfg(test)]
#[path = "key_handler_plan_edit_tests.rs"]
mod plan_edit_tests;

#[cfg(test)]
#[path = "key_handler_queue_scroll_tests.rs"]
mod queue_scroll_tests;

#[cfg(test)]
#[path = "key_handler_file_mention_tests.rs"]
mod file_mention_tests;

#[cfg(test)]
#[path = "key_handler_running_mode_tests.rs"]
mod running_mode_tests;
