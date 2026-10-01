use super::*;

#[test]
fn bang_prefix_returns_bash_action() {
    // `!cmd` + Enter must route to local Bash execution, never Submit.
    let mut input = String::from("!ls");
    let mut cursor = input.chars().count();
    let history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("!ls", cursor);
    let mut queue_scroll: u32 = 0;
    let action = handle_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
        &mut input,
        &mut cursor,
        &history,
        &mut hist_idx,
        false,
        false,
        "act",
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        false,
        false,
        false,
        &mut undo_state,
        &mut queue_scroll,
        &mut None,
    );
    assert!(matches!(action, KeyAction::Bash(ref cmd) if cmd == "ls"));
    assert!(
        input.is_empty(),
        "input must be cleared after Bash dispatch"
    );
}

#[test]
fn bang_prefix_with_spaces_returns_bash() {
    // A leading space right after `!` (e.g. `! echo hi`) is trimmed so the
    // dispatched command is `Bash("echo hi")`, not `Bash(" echo hi")`.
    let mut input = String::from("! echo hi");
    let mut cursor = input.chars().count();
    let history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("! echo hi", cursor);
    let mut queue_scroll: u32 = 0;
    let action = handle_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
        &mut input,
        &mut cursor,
        &history,
        &mut hist_idx,
        false,
        false,
        "act",
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        false,
        false,
        false,
        &mut undo_state,
        &mut queue_scroll,
        &mut None,
    );
    assert!(matches!(action, KeyAction::Bash(ref cmd) if cmd == "echo hi"));
    assert!(
        input.is_empty(),
        "input must be cleared after Bash dispatch"
    );
}

#[test]
fn bare_bang_is_noop() {
    // A lone `!` passes the trim-empty gate but has no command → None; input still cleared.
    let mut input = String::from("!");
    let mut cursor = input.chars().count();
    let history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("!", cursor);
    let mut queue_scroll: u32 = 0;
    let action = handle_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
        &mut input,
        &mut cursor,
        &history,
        &mut hist_idx,
        false,
        false,
        "act",
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        false,
        false,
        false,
        &mut undo_state,
        &mut queue_scroll,
        &mut None,
    );
    assert!(matches!(action, KeyAction::None));
    assert!(input.is_empty(), "input must be cleared even on bare bang");
}

#[test]
fn bang_prefix_works_while_running() {
    // While running, a plain Enter would Steer — but `!cmd` is parsed first,
    // so it still dispatches Bash mid-turn instead of being treated as steer.
    let mut input = String::from("!ls");
    let mut cursor = input.chars().count();
    let history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("!ls", cursor);
    let mut queue_scroll: u32 = 0;
    let action = handle_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
        &mut input,
        &mut cursor,
        &history,
        &mut hist_idx,
        true,
        false,
        "act", // running: would normally route Enter → Steer
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        false,
        false,
        false,
        &mut undo_state,
        &mut queue_scroll,
        &mut None,
    );
    assert!(matches!(action, KeyAction::Bash(ref cmd) if cmd == "ls"));
    assert!(
        input.is_empty(),
        "input must be cleared after Bash dispatch"
    );
}
