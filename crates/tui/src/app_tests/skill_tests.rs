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
/// active and the user switches agent (`/plan` <-> `/act`), `sys_tokens`
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

// ----- resume mirror backfill (run_app startup) -----
// Regression: `run_app` used to hard-code `active_skill`/`active_skill_body`
// to `None` and discard `initial_skill_state`'s body. A resumed `task-plan`
// commit therefore had an empty mirror, so the first idle submit's
// `act_plan_highlight(active_skill)` re-derivation turned the chip gray for
// the whole turn (and the skill-only submit trigger path stayed blind). The
// fix backfills the mirrors from the derived body via
// `skill_display::skill_mirror_from_body`.

/// Chains the exact startup sequence `run_app` performs: derive state from
/// the shared handle (resume read-back) -> backfill the mirror tuple ->
/// re-derive the chip highlight the way the idle submit path does.
#[test]
fn resume_mirror_backfill_keeps_resumed_task_plan_yellow() {
    // A persisted `task-plan` skill body as the store row carries it on
    // resume (directory-style skill with a `> Source:` prefix).
    let body = "> Source: /home/u/.opencoder/skills/task-plan/SKILL.md\n\n\
                plan the work before touching code";
    let skill_handle: std::sync::Arc<std::sync::Mutex<Option<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Some(body.to_string())));

    // 1. initial_skill_state derives body + chip highlight from the handle.
    let (initial_body, _tokens, startup_highlight) = crate::skill_persist::initial_skill_state(
        &skill_handle,
        "act",
        std::path::Path::new("/tmp"),
    );
    assert!(
        startup_highlight,
        "a resumed task-plan commit must start with the chip highlighted"
    );

    // 2. run_app backfills the mirrors from that same body.
    let (active_skill, active_skill_body) =
        crate::skill_display::skill_mirror_from_body(initial_body);

    // 3. the first idle-submit re-derivation keeps the yellow.
    assert_eq!(
        active_skill.as_deref(),
        Some("task-plan"),
        "the mirror must carry the resumed skill name"
    );
    assert_eq!(
        active_skill_body.as_deref(),
        Some(body),
        "the mirror must carry the resumed skill body"
    );
    assert!(
        crate::skill_persist::act_plan_highlight(active_skill.as_deref()),
        "resolve_persist's re-derivation must not grey out a resumed task-plan"
    );
    // Startup highlight and mirror agree on one shared derivation, so the
    // first idle submit cannot disagree with the frame the user already saw.
    assert_eq!(
        startup_highlight,
        crate::skill_persist::act_plan_highlight(active_skill.as_deref()),
        "mirror backfill must agree with the startup highlight derivation"
    );
}

/// A resumed session with NO committed skill must keep both mirrors empty so
/// the chip stays gray (no phantom yellow from the backfill).
#[test]
fn resume_mirror_backfill_keeps_unskilled_session_gray() {
    let skill_handle: std::sync::Arc<std::sync::Mutex<Option<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));

    let (initial_body, _tokens, startup_highlight) = crate::skill_persist::initial_skill_state(
        &skill_handle,
        "act",
        std::path::Path::new("/tmp"),
    );
    assert!(!startup_highlight);
    let (active_skill, active_skill_body) =
        crate::skill_display::skill_mirror_from_body(initial_body);
    assert_eq!(active_skill, None);
    assert_eq!(active_skill_body, None);
    assert!(!crate::skill_persist::act_plan_highlight(
        active_skill.as_deref()
    ));
}

#[path = "skill_tests/lifecycle.rs"]
mod lifecycle;
