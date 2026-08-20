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
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut queue_scroll: u32 = 0;
    let mut file_menu: Option<crate::file_menu::FileMenu> = None;
    let workdir = std::path::Path::new(".");
    handle_key(
        k,
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
        input,
        cursor_idx,
        &history,
        &mut hist_idx,
        running,
        agent,
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        false,
        false,
        &mut undo_state,
        &mut queue_scroll,
        &mut file_menu,
        workdir,
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
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut queue_scroll: u32 = 0;
    let mut file_menu: Option<crate::file_menu::FileMenu> = None;
    let workdir = std::path::Path::new(".");
    handle_key(
        k,
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
        input,
        cursor_idx,
        &history,
        &mut hist_idx,
        false,
        agent,
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        false,
        true,
        &mut undo_state,
        &mut queue_scroll,
        &mut file_menu,
        workdir,
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
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut queue_scroll: u32 = 0;
    let mut file_menu: Option<crate::file_menu::FileMenu> = None;
    let workdir = std::path::Path::new(".");
    handle_key(
        k,
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
        input,
        cursor_idx,
        &history,
        &mut hist_idx,
        true,
        agent,
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        true,
        false,
        &mut undo_state,
        &mut queue_scroll,
        &mut file_menu,
        workdir,
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
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut queue_scroll: u32 = 0;
    let mut file_menu: Option<crate::file_menu::FileMenu> = None;
    let workdir = std::path::Path::new(".");
    handle_key(
        k,
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
        input,
        cursor_idx,
        &history,
        &mut hist_idx,
        false,
        "act",
        &mut scroll,
        &mut follow,
        &mut last_esc,
        skill_menu,
        80,
        2,
        false,
        false,
        &mut undo_state,
        &mut queue_scroll,
        &mut file_menu,
        workdir,
    )
}

/// Full flow: a text recorded by `push_history` (what the Enter/Tab Steer and
/// Queue branches do while the agent is running) is recallable in the input
/// via Up arrow, and Down arrow clears it back to empty.
#[test]
fn up_arrow_recalls_recorded_steer_or_queue_text() {
    let mut history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    crate::app_helpers::push_history(&mut history, &mut hist_idx, "steer while running");

    let mut input = String::new();
    let mut cursor_idx = 0usize;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut queue_scroll: u32 = 0;
    let mut file_menu: Option<crate::file_menu::FileMenu> = None;
    let workdir = std::path::Path::new(".");

    // Up arrow: newest history entry lands in the composer.
    let action = handle_key(
        key(KeyCode::Up, KeyModifiers::NONE),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
        &mut input,
        &mut cursor_idx,
        &history,
        &mut hist_idx,
        true,
        "act",
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        false,
        false,
        &mut undo_state,
        &mut queue_scroll,
        &mut file_menu,
        workdir,
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(input, "steer while running");
    assert_eq!(cursor_idx, "steer while running".chars().count());

    // Down arrow: leaves history browsing back to an empty composer.
    let action = handle_key(
        key(KeyCode::Down, KeyModifiers::NONE),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
        &mut input,
        &mut cursor_idx,
        &history,
        &mut hist_idx,
        true,
        "act",
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        false,
        false,
        &mut undo_state,
        &mut queue_scroll,
        &mut file_menu,
        workdir,
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(input, "");
    assert_eq!(hist_idx, None);
}

mod key_tests;
mod key_tests_quit;
mod skill_tests;
