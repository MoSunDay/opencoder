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

/// Like `run_handle` but exposes the skill-menu state so `$`-trigger and modal
/// behavior can be inspected.
fn run_handle_menu(
    k: KeyEvent,
    input: &mut String,
    cursor_idx: &mut usize,
    skill_menu: &mut Option<SkillMenu>,
    active_skill: Option<&str>,
) -> KeyAction {
    let history: Vec<String> = vec![];
    let mut hist_idx = None;
    let mut show_help = false;
    let mut scroll = 0u16;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    handle_key(
        k, input, cursor_idx, &history, &mut hist_idx,
        false, "act", &mut show_help, &mut scroll, &mut follow,
        &mut last_esc, skill_menu, active_skill,
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
fn enter_while_running_admits_steer() {
    let mut input = String::from("stop and rethink");
    let mut idx = 15;
    let action = run_handle(
        key(KeyCode::Enter, KeyModifiers::NONE),
        &mut input, &mut idx, true, "act",
    );
    assert!(matches!(action, KeyAction::Steer(ref t) if t == "stop and rethink"));
    assert!(input.is_empty());
}

#[test]
fn tab_while_running_admits_queue() {
    let mut input = String::from("next task");
    let mut idx = 9;
    let action = run_handle(
        key(KeyCode::Tab, KeyModifiers::NONE),
        &mut input, &mut idx, true, "act",
    );
    assert!(matches!(action, KeyAction::Queue(ref t) if t == "next task"));
}

#[test]
fn tab_while_idle_submits() {
    let mut input = String::from("hello");
    let mut idx = 5;
    let action = run_handle(
        key(KeyCode::Tab, KeyModifiers::NONE),
        &mut input, &mut idx, false, "act",
    );
    assert!(matches!(action, KeyAction::Submit(ref t) if t == "hello"));
}

#[test]
fn shift_tab_toggles_plan_act() {
    // BackTab = Shift+Tab, the primary mode-switch key (codex-cli style).
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::BackTab, KeyModifiers::SHIFT),
        &mut input, &mut idx, false, "act",
    );
    assert!(matches!(action, KeyAction::SwitchAgent(ref a) if a == "plan"));

    let action2 = run_handle(
        key(KeyCode::BackTab, KeyModifiers::SHIFT),
        &mut input, &mut idx, false, "plan",
    );
    assert!(matches!(action2, KeyAction::SwitchAgent(ref a) if a == "act"));
}

#[test]
fn backtab_without_shift_also_toggles() {
    // Some terminals report Shift+Tab as BackTab with no modifiers.
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::BackTab, KeyModifiers::NONE),
        &mut input, &mut idx, false, "act",
    );
    assert!(matches!(action, KeyAction::SwitchAgent(ref a) if a == "plan"));
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
fn alt_tab_toggles_plan_act() {
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Tab, KeyModifiers::ALT),
        &mut input, &mut idx, false, "act",
    );
    assert!(matches!(action, KeyAction::SwitchAgent(ref a) if a == "plan"));

    let action2 = run_handle(
        key(KeyCode::Tab, KeyModifiers::ALT),
        &mut input, &mut idx, false, "plan",
    );
    assert!(matches!(action2, KeyAction::SwitchAgent(ref a) if a == "act"));
}

#[test]
fn ctrl_o_is_not_steer() {
    // Ctrl+O was removed as a steer trigger (replaced by Enter while running).
    // Verify it does NOT produce a Steer action.
    let mut input = String::from("msg");
    let mut idx = 3;
    let action = run_handle(
        key(KeyCode::Char('o'), KeyModifiers::CONTROL),
        &mut input, &mut idx, true, "act",
    );
    assert!(
        !matches!(action, KeyAction::Steer(_)),
        "Ctrl+O must not steer; got {action:?}"
    );
}

#[test]
fn ctrl_j_is_not_queue() {
    // Ctrl+J was removed as a queue trigger (replaced by Tab while running).
    // Verify it does NOT produce a Queue action.
    let mut input = String::from("msg");
    let mut idx = 3;
    let action = run_handle(
        key(KeyCode::Char('j'), KeyModifiers::CONTROL),
        &mut input, &mut idx, true, "act",
    );
    assert!(
        !matches!(action, KeyAction::Queue(_)),
        "Ctrl+J must not queue; got {action:?}"
    );
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

#[test]
fn ctrl_d_quits() {
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Char('d'), KeyModifiers::CONTROL),
        &mut input, &mut idx, false, "act",
    );
    assert!(matches!(action, KeyAction::Quit), "Ctrl+D must quit");
}

#[test]
fn raw_eot_quits() {
    // Some terminals/crossterm configs deliver Ctrl+D as a bare EOT control
    // char (0x04) without the CONTROL modifier — that path must still quit.
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Char('\u{4}'), KeyModifiers::NONE),
        &mut input, &mut idx, false, "act",
    );
    assert!(matches!(action, KeyAction::Quit), "raw EOT (Ctrl+D) must quit");
}

#[test]
fn raw_etx_quits() {
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Char('\u{3}'), KeyModifiers::NONE),
        &mut input, &mut idx, false, "act",
    );
    assert!(matches!(action, KeyAction::Quit), "raw ETX (Ctrl+C) must quit");
}

#[test]
fn sys_tokens_counts_system_prompt() {
    let dir = std::env::temp_dir();
    let base = crate::app::sys_tokens_for("act", &dir, None);
    assert!(base > 0, "the system prompt must register some tokens");
    // deterministic
    assert_eq!(crate::app::sys_tokens_for("act", &dir, None), base);
    // a skill body adds tokens on top of the base system prompt
    let with_skill = crate::app::sys_tokens_for("act", &dir, Some("extra skill guidance body text"));
    assert!(with_skill > base, "activating a skill must increase the count");
    // unknown agent -> 0 (no panic)
    assert_eq!(crate::app::sys_tokens_for("does-not-exist", &dir, None), 0);
}

#[test]
fn dollar_on_empty_input_opens_skill_menu() {
    let mut input = String::new();
    let mut idx = 0;
    let mut menu: Option<SkillMenu> = None;
    let action = run_handle_menu(
        key(KeyCode::Char('$'), KeyModifiers::NONE),
        &mut input, &mut idx, &mut menu, None,
    );
    assert!(matches!(action, KeyAction::None));
    assert!(menu.is_some(), "`$` on empty input must open the skill menu");
    assert!(input.is_empty(), "`$` must not be inserted into the composer");
}

#[test]
fn dollar_on_non_empty_input_is_literal() {
    let mut input = String::from("pay ");
    let mut idx = 4;
    let mut menu: Option<SkillMenu> = None;
    let action = run_handle_menu(
        key(KeyCode::Char('$'), KeyModifiers::NONE),
        &mut input, &mut idx, &mut menu, None,
    );
    assert!(matches!(action, KeyAction::None));
    assert!(menu.is_none(), "menu must not open when input is non-empty");
    assert_eq!(input, "pay $");
}

#[test]
fn skill_menu_enter_picks_selected_skill() {
    use opencode_core::Skill;
    use std::path::PathBuf;
    let skill = Skill {
        name: "alpha".into(),
        description: "d".into(),
        body: "the body".into(),
        source: PathBuf::from("/x.md"),
    };
    let mut menu = Some(SkillMenu::new(vec![skill], false));
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle_menu(
        key(KeyCode::Enter, KeyModifiers::NONE),
        &mut input, &mut idx, &mut menu, None,
    );
    match action {
        KeyAction::SetSkill(Some((name, body))) => {
            assert_eq!(name, "alpha");
            assert_eq!(body, "the body");
        }
        _ => panic!("expected KeyAction::SetSkill(Some)"),
    }
    assert!(menu.is_none(), "menu must close after a pick");
}

#[test]
fn skill_menu_esc_closes_without_picking() {
    let mut menu = Some(SkillMenu::new(vec![], false));
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle_menu(
        key(KeyCode::Esc, KeyModifiers::NONE),
        &mut input, &mut idx, &mut menu, None,
    );
    assert!(matches!(action, KeyAction::None), "Esc must not pick anything");
    assert!(menu.is_none(), "Esc must close the menu");
}

#[test]
fn skill_menu_intercepts_typing_from_composer() {
    use opencode_core::Skill;
    use std::path::PathBuf;
    let mut menu = Some(SkillMenu::new(
        vec![Skill {
            name: "alpha".into(),
            description: "d".into(),
            body: "b".into(),
            source: PathBuf::from("/x.md"),
        }],
        false,
    ));
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle_menu(
        key(KeyCode::Char('z'), KeyModifiers::NONE),
        &mut input, &mut idx, &mut menu, None,
    );
    assert!(matches!(action, KeyAction::None));
    assert!(input.is_empty(), "typed char must NOT reach the composer while the menu is open");
    assert!(menu.is_some(), "menu stays open while filtering");
}

#[test]
fn skill_menu_clear_row_unsets_skill() {
    use opencode_core::Skill;
    use std::path::PathBuf;
    // has_active=true prepends the "✕ clear" row, selected by default.
    let mut menu = Some(SkillMenu::new(
        vec![Skill {
            name: "alpha".into(),
            description: "d".into(),
            body: "b".into(),
            source: PathBuf::from("/x.md"),
        }],
        true,
    ));
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle_menu(
        key(KeyCode::Enter, KeyModifiers::NONE),
        &mut input, &mut idx, &mut menu, Some("old"),
    );
    assert!(matches!(action, KeyAction::SetSkill(None)), "clear row must yield SetSkill(None)");
    assert!(menu.is_none());
}
