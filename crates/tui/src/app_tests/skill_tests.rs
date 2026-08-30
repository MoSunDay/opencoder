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
    let base = crate::app_helpers::sys_tokens_for("act", &dir, None);
    assert!(base > 0, "the system prompt must register some tokens");
    // deterministic
    assert_eq!(crate::app_helpers::sys_tokens_for("act", &dir, None), base);
    // a plain skill body (no Source prefix, no latent tools) no longer adds
    // tokens: skill bodies moved out of the system prompt, so the count is
    // unchanged until a Source path or latent tool name appears.
    let plain =
        crate::app_helpers::sys_tokens_for("act", &dir, Some("extra skill guidance body text"));
    assert_eq!(
        plain, base,
        "a plain skill body must not change the system-prompt estimate"
    );
    // a Source-prefixed body surfaces the one-line active-skill tail
    // reminder, which does add tokens on top of the base.
    let sourced_body = "> Source: /skills/x/SKILL.md\n\nbody";
    let sourced = crate::app_helpers::sys_tokens_for("act", &dir, Some(sourced_body));
    assert!(
        sourced > base,
        "a Source-prefixed skill body must raise the count (tail reminder)"
    );
    // unknown agent -> 0 (no panic)
    assert_eq!(
        crate::app_helpers::sys_tokens_for("does-not-exist", &dir, None),
        0
    );
}

/// Regression for the agent-switch token-recalculation bug: when a skill is
/// active and the user switches agent (`/sandbox` <-> `/act`), `sys_tokens`
/// is recomputed via `sys_tokens_for(agent, workdir, skill)`. The `skill`
/// argument must be the
/// skill **body** (the stored instruction text), not the skill **name**: the
/// body is what latent-tool unlocking (`tools::latent::unlocked_from_body`)
/// derives from, and it carries the `> Source:` prefix that surfaces the
/// tail reminder. No builtin agent allowlists a latent tool, so the unlock
/// delta is pinned on the exact estimator `sys_tokens_for` feeds the body
/// to, plus an end-to-end body-vs-name check through `sys_tokens_for`.
#[test]
fn sys_tokens_skill_body_unlocks_latent_tools_and_beats_name() {
    // take the shared HOME lock so a concurrent test that mutates HOME can't
    // race a system-prompt build in this test and flake the determinism
    // assertion below (system prompt reads workdir + global instructions).
    let _home = crate::app::app_loop::tests::HOME_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // Tool-schema unlock: a body naming the ssh-pty skill unlocks the
    // ssh_pty schema, a plain body unlocks nothing.
    let all = opencoder_core::Agent {
        name: "all".into(),
        kind: opencoder_core::AgentKind::Act,
        mode: opencoder_core::AgentMode::Primary,
        description: String::new(),
        prompt: String::new(),
        tools: opencoder_core::ToolFilter::All,
    };
    let registry = opencoder_session::tools::registry();
    let plain = "a plain body with no tool names";
    let unlocking = "# ssh-pty skill\n\nUse ssh_pty for persistent SSH.";
    let plain_tokens =
        opencoder_session::tools::estimate_tool_schema_tokens(&all, Some(plain), &registry);
    let unlocking_tokens =
        opencoder_session::tools::estimate_tool_schema_tokens(&all, Some(unlocking), &registry);
    assert!(
        unlocking_tokens > plain_tokens,
        "a body naming a latent tool ({unlocking_tokens}) must exceed a plain \
         body ({plain_tokens}); otherwise the SwitchAgent recalculation \
         under-counts the context meter"
    );
    // End-to-end: a stored body (Source-prefixed) out-estimates the bare
    // skill name — pinning that SwitchAgent passes the body.
    let dir = std::env::temp_dir();
    let by_name = crate::app_helpers::sys_tokens_for("act", &dir, Some("code-review"));
    let by_body = crate::app_helpers::sys_tokens_for(
        "act",
        &dir,
        Some("> Source: /skills/code-review/SKILL.md\n\nReview the diff line by line."),
    );
    assert!(
        by_body > by_name,
        "estimating the stored skill body ({by_body}) must exceed estimating \
         the bare skill name ({by_name})"
    );
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
    let mut file_menu: Option<crate::file_menu::FileMenu> = None;
    let workdir = std::path::Path::new(".");
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
        &mut file_menu,
        workdir,
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
        &mut file_menu,
        workdir,
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
