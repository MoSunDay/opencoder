//! Tests for app::handle_key — split into a separate file to keep app.rs ≤800 lines.

pub(super) use crate::app::{handle_key, KeyAction};
pub(super) use crate::app_helpers::resume_hint;
pub(super) use crate::frame::flash_visible;
pub(super) use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
pub(super) use std::time::Instant;

pub(super) use crate::menu::SkillMenu;

pub(super) fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent::new_with_kind_and_state(code, mods, KeyEventKind::Press, KeyEventState::NONE)
}

pub(super) fn run_handle(
    k: KeyEvent,
    input: &mut String,
    cursor_idx: &mut usize,
    running: bool,
    agent: &str,
) -> KeyAction {
    let history: Vec<String> = vec![];
    let mut hist_idx = None;
    let mut show_help = false;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;
    handle_key(
        k,
        input,
        cursor_idx,
        &history,
        &mut hist_idx,
        running,
        agent,
        &mut show_help,
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        false,
        false,
        &mut undo_state,
        &mut help_scroll,
        &mut queue_scroll,
    )
}

/// Like `run_handle` but with input disabled (subagent-focus view), used to
/// verify that mode-switch chords are correctly suppressed while browsing a
/// subagent.
pub(super) fn run_handle_disabled(
    k: KeyEvent,
    input: &mut String,
    cursor_idx: &mut usize,
    agent: &str,
) -> KeyAction {
    let history: Vec<String> = vec![];
    let mut hist_idx = None;
    let mut show_help = false;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;
    handle_key(
        k,
        input,
        cursor_idx,
        &history,
        &mut hist_idx,
        false,
        agent,
        &mut show_help,
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        false,
        true,
        &mut undo_state,
        &mut help_scroll,
        &mut queue_scroll,
    )
}

/// Like `run_handle` but simulates a *focused running subagent*: input is
/// enabled (`input_disabled = false`) but `subagent_focused = true`.
pub(super) fn run_handle_subagent(
    k: KeyEvent,
    input: &mut String,
    cursor_idx: &mut usize,
    agent: &str,
) -> KeyAction {
    let history: Vec<String> = vec![];
    let mut hist_idx = None;
    let mut show_help = false;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;
    handle_key(
        k,
        input,
        cursor_idx,
        &history,
        &mut hist_idx,
        true,
        agent,
        &mut show_help,
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        true,
        false,
        &mut undo_state,
        &mut help_scroll,
        &mut queue_scroll,
    )
}

/// Like `run_handle` but exposes the skill-menu state so `$`-trigger and modal
/// behavior can be inspected.
pub(super) fn run_handle_menu(
    k: KeyEvent,
    input: &mut String,
    cursor_idx: &mut usize,
    skill_menu: &mut Option<SkillMenu>,
) -> KeyAction {
    let history: Vec<String> = vec![];
    let mut hist_idx = None;
    let mut show_help = false;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;
    handle_key(
        k,
        input,
        cursor_idx,
        &history,
        &mut hist_idx,
        false,
        "act",
        &mut show_help,
        &mut scroll,
        &mut follow,
        &mut last_esc,
        skill_menu,
        80,
        2,
        false,
        false,
        &mut undo_state,
        &mut help_scroll,
        &mut queue_scroll,
    )
}

mod key_tests;
mod skill_tests;
