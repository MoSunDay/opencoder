use super::*;

#[test]
fn sys_tokens_counts_system_prompt() {
    // take the shared HOME lock so a concurrent test that mutates HOME can't
    // race a system-prompt build in this test and flake the determinism
    // assertion below (system prompt reads workdir + global instructions).
    let _home = crate::app::app_loop::tests::HOME_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
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
    // take the shared HOME lock so a concurrent test that mutates HOME can't
    // race a system-prompt build in this test and flake the determinism
    // assertion below (system prompt reads workdir + global instructions).
    let _home = crate::app::app_loop::tests::HOME_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
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
    let mut menu = Some(SkillMenu::new(vec![skill]));
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle_menu(
        key(KeyCode::Enter, KeyModifiers::NONE),
        &mut input,
        &mut idx,
        &mut menu,
    );
    // Picking now inserts a `$name` token at the cursor instead of emitting
    // SetSkill; the skill body is resolved and loaded on submit.
    assert!(
        matches!(action, KeyAction::None),
        "pick must not emit SetSkill"
    );
    assert!(menu.is_none(), "menu must close after a pick");
    // Trailing space separates the token from any text the user types
    // next (prevents `$alpha1` glue that would corrupt the token name).
    assert_eq!(input, "$alpha ");
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
    let mut menu = Some(SkillMenu::new(vec![skill]));
    let mut input = String::from("hello ");
    let mut idx = 6; // end of "hello "
    let action = run_handle_menu(
        key(KeyCode::Enter, KeyModifiers::NONE),
        &mut input,
        &mut idx,
        &mut menu,
    );
    assert!(matches!(action, KeyAction::None));
    assert!(menu.is_none());
    assert_eq!(input, "hello $alpha ");
    assert_eq!(idx, input.chars().count());
}

#[test]
fn skill_menu_esc_closes_without_picking() {
    let mut menu = Some(SkillMenu::new(vec![]));
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle_menu(
        key(KeyCode::Esc, KeyModifiers::NONE),
        &mut input,
        &mut idx,
        &mut menu,
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
    let mut menu = Some(SkillMenu::new(vec![Skill {
        name: "alpha".into(),
        description: "d".into(),
        body: "b".into(),
        source: PathBuf::from("/x.md"),
    }]));
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle_menu(
        key(KeyCode::Char('z'), KeyModifiers::NONE),
        &mut input,
        &mut idx,
        &mut menu,
    );
    assert!(matches!(action, KeyAction::None));
    assert!(
        input.is_empty(),
        "typed char must NOT reach the composer while the menu is open"
    );
    assert!(menu.is_some(), "menu stays open while filtering");
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

#[test]
fn skill_trigger_names_the_active_skill() {
    assert_eq!(
        crate::skill_display::skill_trigger("repo-memory"),
        "The `repo-memory` skill is now active. Begin executing its instructions immediately."
    );
    // The trigger is identical across Submit/Steer/Queue so a pure-skill
    // submission behaves consistently regardless of the submit verb.
    assert!(crate::skill_display::skill_trigger("x").contains("`x`"));
}

#[test]
fn skill_token_display_shows_dollar_token() {
    assert_eq!(
        crate::skill_display::skill_token_display("repo-memory"),
        "$repo-memory",
    );
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
    let ok =
        crate::app::start_turn(&cmd_tx, &mut cancel, UiCmd::Prompt("hi".into(), Vec::new())).await;
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
fn double_esc_while_running_cancels() {
    // Two Esc presses within ESC_CANCEL_WINDOW_MS while running should produce
    // KeyAction::Cancel (hard-abort). The first press records the timestamp;
    // the second, falling inside the window, returns Cancel.
    let history: Vec<String> = vec![];
    let mut input = String::from("draft");
    let mut idx = 5;
    let mut hist_idx = None;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut queue_scroll: u32 = 0;
    let esc = key(KeyCode::Esc, KeyModifiers::NONE);

    let first = handle_key(
        esc,
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
        &mut input,
        &mut idx,
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
    );
    assert!(
        matches!(first, KeyAction::None),
        "first esc is a soft clear"
    );
    assert!(last_esc.is_some(), "first esc records the timestamp");

    let second = handle_key(
        esc,
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
        &mut input,
        &mut idx,
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
    );
    assert!(
        matches!(second, KeyAction::Cancel),
        "double esc within the window must hard-abort"
    );
}

#[test]
fn startup_endpoint_resolves_by_model_prefix_not_legacy_field() {
    use opencoder_core::{Config, ProviderConfig};
    use std::collections::HashMap;
    let mut providers = HashMap::new();
    providers.insert(
        "deepseek".to_string(),
        ProviderConfig {
            base_url: "https://api.deepseek.com/v1".to_string(),
            api_key: Some("dk-key".to_string()),
            model: None,
            headers: Vec::new(),
        },
    );
    let cfg = Config {
        model: "deepseek/deepseek-chat".to_string(),
        // Legacy single-provider field — the value the OLD startup bug picked.
        // Distinct from providers["deepseek"] so a revert to the raw field is
        // caught (it would return the openai url + oai-key instead).
        provider: ProviderConfig {
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: Some("oai-key".to_string()),
            model: None,
            headers: Vec::new(),
        },
        providers,
        ..Default::default()
    };
    let ep = crate::app_helpers::startup_endpoint(&cfg).unwrap();
    assert_eq!(ep.base_url, "https://api.deepseek.com/v1");
    assert_eq!(ep.api_key, "dk-key");
}

#[test]
fn startup_endpoint_falls_back_to_legacy_when_prefix_absent() {
    use opencoder_core::{Config, ProviderConfig};
    use std::collections::HashMap;
    // Model prefix "unknown-svc" is not in providers -> fall back to the
    // legacy top-level provider field (boundary case for the startup seam).
    let cfg = Config {
        model: "unknown-svc/model-x".to_string(),
        provider: ProviderConfig {
            base_url: "https://legacy.example.com/v1".to_string(),
            api_key: Some("legacy-key".to_string()),
            model: None,
            headers: Vec::new(),
        },
        providers: HashMap::new(),
        ..Default::default()
    };
    let ep = crate::app_helpers::startup_endpoint(&cfg).unwrap();
    assert_eq!(ep.base_url, "https://legacy.example.com/v1");
    assert_eq!(ep.api_key, "legacy-key");
}

#[test]
fn size_changed_detects_dimension_change() {
    use crate::app_helpers::size_changed;
    assert!(
        size_changed(Some((80, 24)), (80, 25)),
        "height change must count"
    );
    assert!(
        size_changed(Some((80, 24)), (81, 24)),
        "width change must count"
    );
}

#[test]
fn size_changed_false_when_unchanged() {
    use crate::app_helpers::size_changed;
    assert!(!size_changed(Some((80, 24)), (80, 24)));
}

#[test]
fn size_changed_true_when_no_prior_reading() {
    use crate::app_helpers::size_changed;
    assert!(size_changed(None, (80, 24)));
}

#[test]
fn size_changed_false_for_zero_dimensions() {
    // 0x0 is a transient glitch on minimize/detach; it should not be treated
    // as a real resize target so we avoid spurious autoresize + re-render.
    use crate::app_helpers::size_changed;
    assert!(
        !size_changed(Some((80, 24)), (0, 24)),
        "zero width must not count as a resize"
    );
    assert!(
        !size_changed(Some((80, 24)), (80, 0)),
        "zero height must not count as a resize"
    );
    assert!(
        !size_changed(None, (0, 0)),
        "zero dims from first frame must not count"
    );
}
