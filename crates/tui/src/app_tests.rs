//! Tests for app::handle_key — split into a separate file to keep app.rs ≤800 lines.

use crate::app::{handle_key, KeyAction};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use std::time::Instant;

use crate::menu::SkillMenu;

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent::new_with_kind_and_state(code, mods, KeyEventKind::Press, KeyEventState::NONE)
}

fn run_handle(
    k: KeyEvent,
    input: &mut String,
    cursor_idx: &mut usize,
    running: bool,
    agent: &str,
) -> KeyAction {
    let history: Vec<String> = vec![];
    let mut hist_idx = None;
    let mut show_help = false;
    let mut scroll = 0u16;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    handle_key(
        k, input, cursor_idx, &history, &mut hist_idx,
        running, agent, &mut show_help, &mut scroll, &mut follow,
        &mut last_esc, &mut skill_menu, None,
    )
}

#[test]
fn enter_submits_non_empty_input() {
    let mut input = String::from("hello world");
    let mut idx = 11;
    let action = run_handle(key(KeyCode::Enter, KeyModifiers::NONE), &mut input, &mut idx, false, "act");
    assert!(matches!(action, KeyAction::Submit(ref t) if t == "hello world"));
    assert!(input.is_empty());
    assert_eq!(idx, 0);
}

#[test]
fn enter_empty_input_is_noop() {
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(key(KeyCode::Enter, KeyModifiers::NONE), &mut input, &mut idx, false, "act");
    assert!(matches!(action, KeyAction::None));
}

#[test]
fn ctrl_o_while_running_admits_steer() {
    let mut input = String::from("stop and rethink");
    let mut idx = 15;
    let action = run_handle(
        key(KeyCode::Char('o'), KeyModifiers::CONTROL),
        &mut input, &mut idx, true, "act",
    );
    assert!(matches!(action, KeyAction::Steer(ref t) if t == "stop and rethink"));
    assert!(input.is_empty());
}

#[test]
fn ctrl_o_while_idle_is_noop() {
    let mut input = String::from("hello");
    let mut idx = 5;
    let action = run_handle(
        key(KeyCode::Char('o'), KeyModifiers::CONTROL),
        &mut input, &mut idx, false, "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert!(!input.is_empty(), "input should not be cleared when idle");
}

#[test]
fn ctrl_j_while_running_admits_queue() {
    let mut input = String::from("next task");
    let mut idx = 9;
    let action = run_handle(
        key(KeyCode::Char('j'), KeyModifiers::CONTROL),
        &mut input, &mut idx, true, "act",
    );
    assert!(matches!(action, KeyAction::Queue(ref t) if t == "next task"));
}

#[test]
fn ctrl_t_toggles_plan_act() {
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Char('t'), KeyModifiers::CONTROL),
        &mut input, &mut idx, false, "act",
    );
    assert!(matches!(action, KeyAction::SwitchAgent(ref a) if a == "plan"));

    let action2 = run_handle(
        key(KeyCode::Char('t'), KeyModifiers::CONTROL),
        &mut input, &mut idx, false, "plan",
    );
    assert!(matches!(action2, KeyAction::SwitchAgent(ref a) if a == "act"));
}

#[test]
fn left_right_move_cursor() {
    let mut input = String::from("abc");
    let mut idx = 3;
    run_handle(key(KeyCode::Left, KeyModifiers::NONE), &mut input, &mut idx, false, "act");
    assert_eq!(idx, 2);
    run_handle(key(KeyCode::Left, KeyModifiers::NONE), &mut input, &mut idx, false, "act");
    assert_eq!(idx, 1);
    run_handle(key(KeyCode::Right, KeyModifiers::NONE), &mut input, &mut idx, false, "act");
    assert_eq!(idx, 2);
}

#[test]
fn ctrl_c_quits() {
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Char('c'), KeyModifiers::CONTROL),
        &mut input, &mut idx, false, "act",
    );
    assert!(matches!(action, KeyAction::Quit));
}
