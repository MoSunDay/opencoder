//! Tests for app::handle_key — split into a separate file to keep app.rs ≤800 lines.

use crate::app::{flash_visible, handle_key, KeyAction};
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
        None,
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
        active_skill,
    )
}

#[test]
fn enter_submits_non_empty_input() {
    let mut input = String::from("hello world");
    let mut idx = 11;
    let action = run_handle(
        key(KeyCode::Enter, KeyModifiers::NONE),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::Submit(ref t) if t == "hello world"));
    assert!(input.is_empty());
    assert_eq!(idx, 0);
}

#[test]
fn enter_empty_input_is_noop() {
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Enter, KeyModifiers::NONE),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
}

#[test]
fn enter_while_running_admits_steer() {
    let mut input = String::from("stop and rethink");
    let mut idx = 15;
    let action = run_handle(
        key(KeyCode::Enter, KeyModifiers::NONE),
        &mut input,
        &mut idx,
        true,
        "act",
    );
    assert!(matches!(action, KeyAction::Steer(ref t) if t == "stop and rethink"));
    assert!(input.is_empty());
}

#[test]
fn enter_with_shift_inserts_newline() {
    let mut input = String::from("hello");
    let mut idx = 5;
    let action = run_handle(
        key(KeyCode::Enter, KeyModifiers::SHIFT),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(input, "hello\n");
    assert_eq!(idx, 6);
}

#[test]
fn enter_with_alt_inserts_newline() {
    let mut input = String::from("hi");
    let mut idx = 2;
    let action = run_handle(
        key(KeyCode::Enter, KeyModifiers::ALT),
        &mut input,
        &mut idx,
        true,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(input, "hi\n");
}

#[test]
fn ctrl_j_inserts_newline() {
    let mut input = String::from("ab");
    let mut idx = 2;
    let action = run_handle(
        key(KeyCode::Char('j'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(input, "ab\n");
}

#[test]
fn tab_while_running_admits_queue() {
    let mut input = String::from("next task");
    let mut idx = 9;
    let action = run_handle(
        key(KeyCode::Tab, KeyModifiers::NONE),
        &mut input,
        &mut idx,
        true,
        "act",
    );
    assert!(matches!(action, KeyAction::Queue(ref t) if t == "next task"));
}

#[test]
fn tab_while_idle_submits() {
    let mut input = String::from("hello");
    let mut idx = 5;
    let action = run_handle(
        key(KeyCode::Tab, KeyModifiers::NONE),
        &mut input,
        &mut idx,
        false,
        "act",
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
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::SwitchAgent(ref a) if a == "plan"));

    let action2 = run_handle(
        key(KeyCode::BackTab, KeyModifiers::SHIFT),
        &mut input,
        &mut idx,
        false,
        "plan",
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
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::SwitchAgent(ref a) if a == "plan"));
}

#[test]
fn ctrl_t_toggles_plan_act() {
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Char('t'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::SwitchAgent(ref a) if a == "plan"));

    let action2 = run_handle(
        key(KeyCode::Char('t'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "plan",
    );
    assert!(matches!(action2, KeyAction::SwitchAgent(ref a) if a == "act"));
}

#[test]
fn alt_tab_toggles_plan_act() {
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Tab, KeyModifiers::ALT),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::SwitchAgent(ref a) if a == "plan"));

    let action2 = run_handle(
        key(KeyCode::Tab, KeyModifiers::ALT),
        &mut input,
        &mut idx,
        false,
        "plan",
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
        &mut input,
        &mut idx,
        true,
        "act",
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
        &mut input,
        &mut idx,
        true,
        "act",
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
    run_handle(
        key(KeyCode::Left, KeyModifiers::NONE),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert_eq!(idx, 2);
    run_handle(
        key(KeyCode::Left, KeyModifiers::NONE),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert_eq!(idx, 1);
    run_handle(
        key(KeyCode::Right, KeyModifiers::NONE),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert_eq!(idx, 2);
}

#[test]
fn ctrl_c_quits() {
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Char('c'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::Quit));
}

#[test]
fn ctrl_d_quits() {
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Char('d'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
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
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(
        matches!(action, KeyAction::Quit),
        "raw EOT (Ctrl+D) must quit"
    );
}

#[test]
fn raw_etx_quits() {
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Char('\u{3}'), KeyModifiers::NONE),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(
        matches!(action, KeyAction::Quit),
        "raw ETX (Ctrl+C) must quit"
    );
}

#[test]
fn kitty_ctrl_d_quits() {
    // Under Kitty keyboard protocol (DISAMBIGUATE_ESCAPE_CODES) crossterm
    // reports Ctrl+D as Char('\u{4}') WITH the CONTROL modifier — this must
    // still quit (regression: was swallowed by the CONTROL match arm).
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Char('\u{4}'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::Quit), "Kitty Ctrl+D must quit");
}

#[test]
fn kitty_ctrl_c_quits() {
    // Same Kitty-protocol path for Ctrl+C (Char('\u{3}') + CONTROL).
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Char('\u{3}'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::Quit), "Kitty Ctrl+C must quit");
}

#[test]
fn sys_tokens_counts_system_prompt() {
    let dir = std::env::temp_dir();
    let base = crate::app::sys_tokens_for("act", &dir, None);
    assert!(base > 0, "the system prompt must register some tokens");
    // deterministic
    assert_eq!(crate::app::sys_tokens_for("act", &dir, None), base);
    // a skill body adds tokens on top of the base system prompt
    let with_skill =
        crate::app::sys_tokens_for("act", &dir, Some("extra skill guidance body text"));
    assert!(
        with_skill > base,
        "activating a skill must increase the count"
    );
    // unknown agent -> 0 (no panic)
    assert_eq!(crate::app::sys_tokens_for("does-not-exist", &dir, None), 0);
}

/// Regression for the SwitchAgent token-recalculation bug (`app.rs`,
/// `KeyAction::SwitchAgent`): when a skill is active and the user switches
/// agent mode (plan <-> act), `sys_tokens` is recomputed via
/// `sys_tokens_for(agent, workdir, skill)`. The `skill` argument must be the
/// skill **body** (the injected instruction text), not the skill **name** —
/// otherwise the "ctx N%" meter under-counts, estimating a short label instead
/// of the (potentially long) instruction. This pins the contract that call
/// relies on: a long body must dominate a short name by a wide margin, so
/// passing the body is observably correct.
#[test]
fn sys_tokens_skill_body_dominates_skill_name() {
    let dir = std::env::temp_dir();
    // A realistic short skill name vs. a long instruction body.
    let name = "code-review";
    let body = "x".repeat(500);
    let by_name = crate::app::sys_tokens_for("act", &dir, Some(name));
    let by_body = crate::app::sys_tokens_for("act", &dir, Some(&body));
    assert!(
        by_body > by_name + 100,
        "estimating the skill body ({by_body}) must far exceed estimating the \
         skill name ({by_name}); otherwise the SwitchAgent recalculation \
         under-counts the context meter"
    );
    // Sanity: the body-based estimate also exceeds the no-skill baseline.
    let base = crate::app::sys_tokens_for("act", &dir, None);
    assert!(by_body > base, "a long skill body must raise the count");
}

#[test]
fn dollar_on_empty_input_opens_skill_menu() {
    let mut input = String::new();
    let mut idx = 0;
    let mut menu: Option<SkillMenu> = None;
    let action = run_handle_menu(
        key(KeyCode::Char('$'), KeyModifiers::NONE),
        &mut input,
        &mut idx,
        &mut menu,
        None,
    );
    assert!(matches!(action, KeyAction::None));
    assert!(
        menu.is_some(),
        "`$` on empty input must open the skill menu"
    );
    assert!(
        input.is_empty(),
        "`$` must not be inserted into the composer"
    );
}

#[test]
fn dollar_on_non_empty_input_is_literal() {
    let mut input = String::from("pay ");
    let mut idx = 4;
    let mut menu: Option<SkillMenu> = None;
    let action = run_handle_menu(
        key(KeyCode::Char('$'), KeyModifiers::NONE),
        &mut input,
        &mut idx,
        &mut menu,
        None,
    );
    assert!(matches!(action, KeyAction::None));
    assert!(menu.is_none(), "menu must not open when input is non-empty");
    assert_eq!(input, "pay $");
}

#[test]
fn skill_menu_enter_picks_selected_skill() {
    use opencoder_core::Skill;
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
        &mut input,
        &mut idx,
        &mut menu,
        None,
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
        &mut input,
        &mut idx,
        &mut menu,
        None,
    );
    assert!(
        matches!(action, KeyAction::None),
        "Esc must not pick anything"
    );
    assert!(menu.is_none(), "Esc must close the menu");
}

#[test]
fn skill_menu_intercepts_typing_from_composer() {
    use opencoder_core::Skill;
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
        &mut input,
        &mut idx,
        &mut menu,
        None,
    );
    assert!(matches!(action, KeyAction::None));
    assert!(
        input.is_empty(),
        "typed char must NOT reach the composer while the menu is open"
    );
    assert!(menu.is_some(), "menu stays open while filtering");
}

#[test]
fn skill_menu_clear_row_unsets_skill() {
    use opencoder_core::Skill;
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
        &mut input,
        &mut idx,
        &mut menu,
        Some("old"),
    );
    assert!(
        matches!(action, KeyAction::SetSkill(None)),
        "clear row must yield SetSkill(None)"
    );
    assert!(menu.is_none());
}

#[test]
fn flash_visible_within_window() {
    assert!(flash_visible(10, 11, 5));
    assert!(flash_visible(10, 14, 5));
}

#[test]
fn flash_visible_expired() {
    assert!(!flash_visible(10, 15, 5));
    assert!(!flash_visible(10, 99, 5));
}

#[test]
fn flash_visible_handles_wraparound() {
    // start near u32::MAX; `now` wraps past 0. Ages 0..4 -> visible, 5 -> expired.
    assert!(flash_visible(u32::MAX, u32::MAX, 5));
    assert!(flash_visible(u32::MAX, 0, 5));
    assert!(flash_visible(u32::MAX, 3, 5));
    assert!(!flash_visible(u32::MAX, 4, 5));
    assert!(!flash_visible(u32::MAX, 99, 5));
}

/// `start_turn` must report failure when the worker command channel has no
/// consumer — the exact signature of a dead worker task (panic or unexpected
/// exit). The main loop relies on this `false` to surface a marker and exit
/// instead of silently queuing into a void and spinning the spinner forever.
#[tokio::test]
async fn start_turn_reports_false_when_worker_is_dead() {
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use crate::worker::UiCmd;

    let (cmd_tx, cmd_rx) = mpsc::channel::<UiCmd>(8);
    drop(cmd_rx); // worker gone — channel closed
    let mut cancel = CancellationToken::new();
    let ok = crate::app::start_turn(&cmd_tx, &mut cancel, UiCmd::Prompt("hi".into())).await;
    assert!(
        !ok,
        "start_turn must return false when the worker channel is closed"
    );
}

/// `worker_dead` surfaces a visible marker so the user understands the engine
/// stopped (rather than an unexplained freeze).
#[test]
fn worker_dead_pushes_a_marker() {
    let mut chat = crate::chat::ChatView::default();
    crate::app::worker_dead(&mut chat);
    let text = crate::chat::block_text(&chat);
    assert!(
        text.contains("worker stopped"),
        "expected a worker-stopped marker; got: {text}"
    );
}
