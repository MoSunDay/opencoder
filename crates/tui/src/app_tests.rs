//! Tests for app::handle_key — split into a separate file to keep app.rs ≤800 lines.

use crate::app::{flash_visible, handle_key, resume_hint, KeyAction};
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
        80,
        2,
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
        80,
        2,
    )
}

#[test]
fn resume_hint_is_copyable_command() {
    assert_eq!(
        resume_hint("01ABC"),
        "resume with: opencoder -s 01ABC"
    );
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
fn ctrl_c_does_not_quit() {
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Char('c'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(
        !matches!(action, KeyAction::Quit),
        "Ctrl+C must not quit anymore"
    );
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
fn raw_etx_does_not_quit() {
    // Ctrl+C (ETX, 0x03) no longer quits — it is swallowed so the raw control
    // char is not inserted into the input buffer.
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
        !matches!(action, KeyAction::Quit),
        "raw ETX (Ctrl+C) must not quit"
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
fn kitty_ctrl_c_does_not_quit() {
    // Kitty-protocol path for Ctrl+C (Char('\u{3}') + CONTROL) no longer quits.
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Char('\u{3}'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(
        !matches!(action, KeyAction::Quit),
        "Kitty Ctrl+C must not quit"
    );
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
fn dollar_anywhere_opens_skill_menu() {
    // `$` triggers the skill picker regardless of cursor position or existing
    // text — the `$` itself is consumed (never inserted into the composer).
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
    assert!(
        menu.is_some(),
        "`$` must open the skill menu even on non-empty input"
    );
    assert_eq!(input, "pay ", "the `$` must be consumed, not inserted");
    assert_eq!(idx, 4, "cursor must stay where it was");
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
    // Picking now inserts a `{$name}` token at the cursor instead of emitting
    // SetSkill; the skill body is resolved and loaded on submit.
    assert!(
        matches!(action, KeyAction::None),
        "pick must not emit SetSkill"
    );
    assert!(menu.is_none(), "menu must close after a pick");
    assert_eq!(input, "{$alpha}");
    assert_eq!(
        idx,
        input.chars().count(),
        "cursor must sit just after the inserted token"
    );
}

#[test]
fn pick_inserts_token_at_cursor_mid_text() {
    use opencoder_core::Skill;
    use std::path::PathBuf;
    let skill = Skill {
        name: "alpha".into(),
        description: "d".into(),
        body: "b".into(),
        source: PathBuf::from("/x.md"),
    };
    let mut menu = Some(SkillMenu::new(vec![skill], false));
    let mut input = String::from("hello ");
    let mut idx = 6; // end of "hello "
    let action = run_handle_menu(
        key(KeyCode::Enter, KeyModifiers::NONE),
        &mut input,
        &mut idx,
        &mut menu,
        None,
    );
    assert!(matches!(action, KeyAction::None));
    assert!(menu.is_none());
    assert_eq!(input, "hello {$alpha}");
    assert_eq!(idx, input.chars().count());
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

#[test]
fn ctrl_a_moves_cursor_to_start() {
    let mut input = String::from("hello");
    let mut idx = 4;
    let action = run_handle(
        key(KeyCode::Char('a'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(idx, 0, "Ctrl+A must move cursor to the first char");
    assert_eq!(input, "hello", "Ctrl+A must not mutate the input");
}

#[test]
fn ctrl_e_moves_cursor_to_end() {
    let mut input = String::from("hello");
    let mut idx = 1;
    let action = run_handle(
        key(KeyCode::Char('e'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(idx, 5, "Ctrl+E must move cursor past the last char");
    assert_eq!(input, "hello", "Ctrl+E must not mutate the input");
}

#[test]
fn ctrl_a_e_on_empty_input_stay_at_zero() {
    let mut input = String::new();
    let mut idx = 0;
    run_handle(
        key(KeyCode::Char('a'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert_eq!(idx, 0);
    run_handle(
        key(KeyCode::Char('e'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert_eq!(idx, 0, "Ctrl+E on empty input must stay at 0");
}

#[test]
fn ctrl_a_e_handle_multibyte_chars() {
    // "héllo" is 5 chars but 6 bytes; cursor_idx is a char index.
    let mut input = String::from("héllo");
    let mut idx = 3;
    run_handle(
        key(KeyCode::Char('a'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert_eq!(idx, 0);
    run_handle(
        key(KeyCode::Char('e'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert_eq!(idx, 5, "Ctrl+E must land at char count, not byte length");
}

#[test]
fn ctrl_w_deletes_word_before_cursor() {
    let mut input = String::from("hello world");
    let mut idx = 11;
    let action = run_handle(
        key(KeyCode::Char('w'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(input, "hello ");
    assert_eq!(idx, 6, "Ctrl+W must move cursor to end of remaining text");
}

#[test]
fn ctrl_w_at_start_is_noop() {
    let mut input = String::from("hello");
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Char('w'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(input, "hello", "Ctrl+W at start must not mutate input");
    assert_eq!(idx, 0);
}

#[test]
fn ctrl_w_empty_input_is_noop() {
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Char('w'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert!(input.is_empty());
    assert_eq!(idx, 0);
}

#[test]
fn ctrl_w_does_not_cross_newline() {
    let mut input = String::from("line1\nline2");
    let mut idx = 11;
    let action = run_handle(
        key(KeyCode::Char('w'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(input, "line1\n", "Ctrl+W must not delete across newlines");
    assert_eq!(idx, 6);
}

#[test]
fn ctrl_w_trailing_whitespace() {
    // "hello   |" → "" — Ctrl+W deletes word + trailing whitespace (bash behavior)
    let mut input = String::from("hello   ");
    let mut idx = 8;
    let action = run_handle(
        key(KeyCode::Char('w'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(input, "");
    assert_eq!(idx, 0);
}

#[test]
fn ctrl_w_multibyte_chars() {
    // "你好 world|" → "你好 |"
    let mut input = String::from("你好 world");
    let mut idx = 8;
    let action = run_handle(
        key(KeyCode::Char('w'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(input, "你好 ");
    assert_eq!(idx, 3, "cursor must be at char boundary after 你好 ");
}

// ---- apply_skill_tokens tests ----
// `apply_skill_tokens` calls `discover_skills()` which reads `~/.opencoder/skills`,
// so these tests serialize `HOME` mutations via a dedicated mutex (mirroring the
// pattern in session/tests/prompt.rs) and point HOME at a tempdir.

static APPTEST_HOME_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn with_home<R>(home: &std::path::Path, f: impl FnOnce() -> R) -> R {
    let _guard = APPTEST_HOME_MUTEX.lock().unwrap();
    let old = std::env::var_os("HOME");
    std::env::set_var("HOME", home);
    let result = f();
    match old {
        Some(h) => std::env::set_var("HOME", h),
        None => std::env::remove_var("HOME"),
    }
    result
}

/// Create a tempdir whose `~/.opencoder/skills/<name>.md` contains a skill
/// with the given body, returning the tempdir (keep alive for the test).
fn skill_tempdir(name: &str, body: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let skills = dir.path().join(".opencoder").join("skills");
    std::fs::create_dir_all(&skills).unwrap();
    std::fs::write(skills.join(format!("{name}.md")), body).unwrap();
    dir
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn apply_skill_tokens_resolves_and_activates_known_skill() {
    let dir = skill_tempdir("alpha", "the alpha body");
    let skill_handle: std::sync::Arc<std::sync::Mutex<Option<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let mut active_skill = None;
    let mut active_skill_body = None;
    let mut sys_tokens = 0u64;
    let workdir = std::path::PathBuf::from("/tmp");

    let (clean, unresolved) = with_home(dir.path(), || {
        crate::app_helpers::apply_skill_tokens(
            "hello {$alpha} world",
            &mut active_skill,
            &mut active_skill_body,
            &mut sys_tokens,
            "act",
            &workdir,
            &skill_handle,
        )
    });

    // Token stripped from clean text; name not unresolved.
    assert_eq!(clean, "hello  world");
    assert!(unresolved.is_empty(), "known skill must not be unresolved");
    // Skill activated (sticky display + body).
    assert_eq!(active_skill.as_deref(), Some("alpha"));
    assert_eq!(active_skill_body.as_deref(), Some("the alpha body"));
    assert!(
        sys_tokens > 0,
        "sys_tokens must be recomputed with the skill body"
    );
    // The shared skill_handle (session.skill_prompt) is updated in-place.
    assert_eq!(
        skill_handle.lock().unwrap().as_deref(),
        Some("the alpha body"),
        "skill_handle must hold the resolved body"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn apply_skill_tokens_reports_unknown_skill() {
    let dir = skill_tempdir("alpha", "alpha body");
    let skill_handle: std::sync::Arc<std::sync::Mutex<Option<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let mut active_skill = None;
    let mut active_skill_body = None;
    let mut sys_tokens = 0u64;
    let workdir = std::path::PathBuf::from("/tmp");

    let (clean, unresolved) = with_home(dir.path(), || {
        crate::app_helpers::apply_skill_tokens(
            "go {$ghost} now",
            &mut active_skill,
            &mut active_skill_body,
            &mut sys_tokens,
            "act",
            &workdir,
            &skill_handle,
        )
    });

    assert_eq!(clean, "go  now");
    assert_eq!(unresolved, vec!["ghost".to_string()]);
    // No skill resolved -> active skill untouched, sys_tokens unchanged.
    assert!(active_skill.is_none());
    assert!(active_skill_body.is_none());
    assert_eq!(
        sys_tokens, 0,
        "sys_tokens must not change when nothing resolves"
    );
    assert!(
        skill_handle.lock().unwrap().is_none(),
        "skill_handle must not be written when nothing resolves"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn apply_skill_tokens_no_tokens_leaves_skill_untouched() {
    let dir = skill_tempdir("alpha", "alpha body");
    let skill_handle: std::sync::Arc<std::sync::Mutex<Option<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Some("prior body".to_string())));
    let mut active_skill = Some("prior".to_string());
    let mut active_skill_body = Some("prior body".to_string());
    let mut sys_tokens = 999u64;
    let workdir = std::path::PathBuf::from("/tmp");

    let (clean, unresolved) = with_home(dir.path(), || {
        crate::app_helpers::apply_skill_tokens(
            "plain text no tokens",
            &mut active_skill,
            &mut active_skill_body,
            &mut sys_tokens,
            "act",
            &workdir,
            &skill_handle,
        )
    });

    // No tokens -> text unchanged, nothing unresolved, sticky skill preserved.
    assert_eq!(clean, "plain text no tokens");
    assert!(unresolved.is_empty());
    assert_eq!(active_skill.as_deref(), Some("prior"));
    assert_eq!(active_skill_body.as_deref(), Some("prior body"));
    assert_eq!(sys_tokens, 999, "sys_tokens must not be recomputed");
    assert_eq!(
        skill_handle.lock().unwrap().as_deref(),
        Some("prior body"),
        "skill_handle must be untouched when no tokens present"
    );
}

