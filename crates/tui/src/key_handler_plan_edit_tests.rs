//! Unit tests for `handle_key` history / plan-edit / subagent behavior:
//! history cycling (`move_hist`), Shift+I plan-edit entry gating, soft-wrap
//! row navigation, and Enter -> Steer / SubagentSteer dispatch. Extracted from
//! `key_handler.rs` to keep it under the 800-line file-size cap.

use super::*;

#[test]
fn move_hist_down_does_not_clear_input_when_not_browsing() {
    let history = vec!["previous command".to_string()];
    let mut hist_idx = None;
    let mut input = "typing something".to_string();
    let mut cursor = 5;
    move_hist(&history, &mut hist_idx, &mut input, &mut cursor, 1);
    assert_eq!(
        input, "typing something",
        "Down should not clear input when not browsing history"
    );
    assert_eq!(hist_idx, None, "hist_idx should remain None");
}

#[test]
fn move_hist_up_loads_previous_entry() {
    let history = vec!["cmd1".to_string(), "cmd2".to_string()];
    let mut hist_idx = None;
    let mut input = "current".to_string();
    let mut cursor = 0;
    move_hist(&history, &mut hist_idx, &mut input, &mut cursor, -1);
    assert_eq!(
        input, "cmd2",
        "Up should load the most recent history entry"
    );
    assert_eq!(hist_idx, Some(1));
}

#[test]
fn move_hist_down_after_up_restores_blank() {
    let history = vec!["cmd1".to_string()];
    let mut hist_idx = None;
    let mut input = "original".to_string();
    let mut cursor = 0;
    // Up loads history
    move_hist(&history, &mut hist_idx, &mut input, &mut cursor, -1);
    assert_eq!(input, "cmd1");
    // Down goes past the end → clears
    move_hist(&history, &mut hist_idx, &mut input, &mut cursor, 1);
    assert_eq!(input, "", "Down past newest should clear input");
    assert_eq!(hist_idx, None);
}

#[test]
fn shift_i_in_plan_mode_idle_enters_plan_edit() {
    let mut input = String::new();
    let mut cursor = 0usize;
    let history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut show_help = false;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;

    // Shift+I (uppercase I) on empty input while idle in plan mode enters
    // the plan-text editor.
    let action = handle_key(
        KeyEvent::new(KeyCode::Char('I'), KeyModifiers::NONE),
        &mut input,
        &mut cursor,
        &history,
        &mut hist_idx,
        false,
        "plan",
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
    );
    assert!(matches!(action, KeyAction::EnterPlanEdit));
    assert!(
        input.is_empty(),
        "input should be untouched on EnterPlanEdit"
    );
}

#[test]
fn shift_i_in_act_mode_does_not_enter_plan_edit() {
    let mut input = String::new();
    let mut cursor = 0usize;
    let history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut show_help = false;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;

    // Shift+I in act mode is a plain char insertion, not plan-edit entry.
    let action = handle_key(
        KeyEvent::new(KeyCode::Char('I'), KeyModifiers::NONE),
        &mut input,
        &mut cursor,
        &history,
        &mut hist_idx,
        false,
        "act",
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
    );
    assert!(!matches!(action, KeyAction::EnterPlanEdit));
    assert_eq!(input, "I", "should insert the character 'I'");
}

#[test]
fn shift_i_while_running_does_not_enter_plan_edit() {
    let mut input = String::new();
    let mut cursor = 0usize;
    let history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut show_help = false;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;

    // Even in plan mode, Shift+I while running just inserts the char.
    let action = handle_key(
        KeyEvent::new(KeyCode::Char('I'), KeyModifiers::NONE),
        &mut input,
        &mut cursor,
        &history,
        &mut hist_idx,
        true,
        "plan",
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
    );
    assert!(!matches!(action, KeyAction::EnterPlanEdit));
    assert_eq!(input, "I", "should insert the character 'I'");
}

#[test]
fn shift_i_with_nonempty_input_does_not_enter_plan_edit() {
    let mut input = "hello".to_string();
    let mut cursor = 5usize;
    let history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut show_help = false;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;

    // Once the user has started typing, Shift+I resumes normal insertion.
    let action = handle_key(
        KeyEvent::new(KeyCode::Char('I'), KeyModifiers::NONE),
        &mut input,
        &mut cursor,
        &history,
        &mut hist_idx,
        false,
        "plan",
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
    );
    assert!(!matches!(action, KeyAction::EnterPlanEdit));
    assert_eq!(input, "helloI", "should append the character 'I'");
}

#[test]
fn lowercase_i_in_plan_mode_inserts_normally() {
    let mut input = String::new();
    let mut cursor = 0usize;
    let history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut show_help = false;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;

    // Lowercase 'i' is unaffected by the plan-edit intercept: plain insert.
    let action = handle_key(
        KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
        &mut input,
        &mut cursor,
        &history,
        &mut hist_idx,
        false,
        "plan",
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
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(input, "i", "lowercase i should be inserted into input");
    assert_eq!(cursor, 1);
}

#[test]
fn up_down_navigate_soft_wrapped_rows() {
    // A long single line that soft-wraps into multiple visual rows at a
    // narrow width. Up/Down should move the cursor across visual rows
    // (not cycle history) because display_rows > 1.
    let mut input = "hello world this is a very long line".to_string();
    let mut cursor = input.chars().count();
    let history: Vec<String> = vec!["past command".to_string()];
    let mut hist_idx: Option<usize> = None;
    let mut show_help = false;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;
    // narrow width (inner_w=10) forces wrapping into multiple rows
    let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
    let res = handle_key(
        up,
        &mut input,
        &mut cursor,
        &history,
        &mut hist_idx,
        false,
        "act",
        &mut show_help,
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        10,
        2,
        false,
        false,
        &mut undo_state,
        &mut help_scroll,
            &mut queue_scroll,
    );
    assert!(matches!(res, KeyAction::None));
    // History was NOT cycled (input unchanged, hist_idx still None)
    assert_eq!(hist_idx, None);
    assert_eq!(input, "hello world this is a very long line");
    // Cursor moved up (decreased) from the end
    assert!(cursor < "hello world this is a very long line".chars().count());
}

#[test]
fn enter_produces_subagent_steer_when_focused() {
    let mut input = String::from("steer the subagent");
    let mut cursor = input.len();
    let history: Vec<String> = vec![];
    let mut hist_idx = None;
    let mut show_help = false;
    let mut scroll = 0u32;
    let mut follow = false;
    let mut last_esc = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;

    // Enter with a running subagent focused produces SubagentSteer (not
    // Steer/Submit), and the input line is cleared.
    let action = handle_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut input,
        &mut cursor,
        &history,
        &mut hist_idx,
        true, // running
        "act",
        &mut show_help,
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        78,    // inner_w
        2,     // prompt_w
        true,  // subagent_focused
        false, // input_disabled
        &mut undo_state,
        &mut help_scroll,
            &mut queue_scroll,
    );

    assert!(matches!(action, KeyAction::SubagentSteer(ref t) if t == "steer the subagent"));
    assert!(input.is_empty(), "input cleared after steer submit");
    assert_eq!(cursor, 0);
}

#[test]
fn enter_produces_steer_when_running_and_not_subagent_focused() {
    // When no subagent is focused but the parent is running, Enter should
    // produce a plain Steer (the default behaviour), NOT SubagentSteer.
    let mut input = String::from("steer the parent");
    let mut cursor = input.len();
    let history: Vec<String> = vec![];
    let mut hist_idx = None;
    let mut show_help = false;
    let mut scroll = 0u32;
    let mut follow = false;
    let mut last_esc = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;

    let action = handle_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut input,
        &mut cursor,
        &history,
        &mut hist_idx,
        true,
        "act",
        &mut show_help,
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        78,
        2,
        false, // subagent_focused
        false,
        &mut undo_state,
        &mut help_scroll,
            &mut queue_scroll,
    );

    assert!(matches!(action, KeyAction::Steer(ref t) if t == "steer the parent"));
}
