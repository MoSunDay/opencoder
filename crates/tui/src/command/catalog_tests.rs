use super::*;

#[test]
fn complete_registered_commands_select_their_own_action() {
    for (name, _) in COMMANDS {
        let mut menu = CommandMenu::new();
        for character in name.chars() {
            menu.on_char(character);
        }
        assert_eq!(menu.selected_name(), Some(*name), "query {name}");
        assert_eq!(menu.selected_action(), dispatch(name), "query {name}");
    }
}

#[test]
fn command_prefix_precedes_an_earlier_substring_match() {
    let mut menu = CommandMenu::new();
    for character in "ac".chars() {
        menu.on_char(character);
    }
    assert_eq!(menu.selected_name(), Some("/act"));
    assert_eq!(menu.selected_action(), Some(SlashAction::Act));
}

#[test]
fn parse_known_commands() {
    assert_eq!(parse("/config"), Some(SlashAction::Config));
    assert_eq!(parse("/cfg"), Some(SlashAction::Config));
    assert_eq!(parse("/task"), Some(SlashAction::Task));
    assert_eq!(parse("/t"), Some(SlashAction::Task));
    assert_eq!(parse("/compact"), Some(SlashAction::Compact));
    assert_eq!(parse("/c"), Some(SlashAction::Compact));
    assert_eq!(parse("/cli"), Some(SlashAction::Cli));
    assert_eq!(parse("/mcp"), Some(SlashAction::Mcp));
    assert_eq!(parse("/skill"), Some(SlashAction::Skill));
    assert_eq!(parse("/sk"), Some(SlashAction::Skill));
    // Agent commands are not exposed in the TUI yet.
    assert_eq!(parse("/agent"), None);
    assert_eq!(parse("/agent writer"), None);
    assert_eq!(parse("/"), Some(SlashAction::Task));
    assert_eq!(parse("/unknown"), None);
    assert_eq!(parse("hello"), None);
    assert_eq!(parse(" /config "), Some(SlashAction::Config));
}

#[test]
fn agent_entry_is_hidden_from_picker() {
    assert!(!COMMANDS.iter().any(|(name, _)| *name == "/agent"));
    let mut m = CommandMenu::new();
    m.paste("agent");
    assert_ne!(m.selected_action(), Some(SlashAction::Agent));
}

#[test]
fn menu_filters_by_query() {
    let mut m = CommandMenu::new();
    assert!(
        m.visible_count() >= 3,
        "all commands visible with empty query"
    );
    for c in "config".chars() {
        m.on_char(c);
    }
    assert_eq!(m.visible_count(), 1, "only /config matches 'config'");
    assert_eq!(m.selected_action(), Some(SlashAction::Config));
}

#[test]
fn menu_filters_compact() {
    let mut m = CommandMenu::new();
    for c in "compact".chars() {
        m.on_char(c);
    }
    assert_eq!(m.visible_count(), 1, "only /compact matches 'compact'");
    assert_eq!(m.selected_action(), Some(SlashAction::Compact));
}

#[test]
fn empty_query_defaults_to_task() {
    let m = CommandMenu::new();
    assert_eq!(
        m.selected_action(),
        Some(SlashAction::Task),
        "first row is /task"
    );
}

#[test]
fn paste_appends_to_query_and_refilters() {
    let mut m = CommandMenu::new();
    let all = m.visible_count();
    assert!(m.query().is_empty());
    m.paste("task");
    assert_eq!(m.query(), "task");
    assert!(m.visible_count() >= 1, "filter should still match 'task'");
    assert!(
        m.visible_count() < all,
        "refilter should narrow the visible list"
    );
}

#[test]
fn parse_control_commands() {
    assert_eq!(parse("/act"), Some(SlashAction::Act));
    assert_eq!(parse("/plan"), Some(SlashAction::Plan));
    assert_eq!(parse("/act_clear_context"), Some(SlashAction::ClearContext));
    // Legacy alias of /act_clear_context must keep parsing.
    assert_eq!(parse("/clear_context"), Some(SlashAction::ClearContext));
    assert_eq!(parse(" /plan "), Some(SlashAction::Plan));
}

#[test]
fn control_cmd_string_maps_correctly() {
    assert_eq!(control_cmd_string(&SlashAction::Act), Some("/act"));
    assert_eq!(control_cmd_string(&SlashAction::Plan), Some("/plan"));
    assert_eq!(
        control_cmd_string(&SlashAction::ClearContext),
        Some("/act_clear_context")
    );
    assert_eq!(control_cmd_string(&SlashAction::Task), None);
    assert_eq!(control_cmd_string(&SlashAction::Compact), None);
    assert_eq!(control_cmd_string(&SlashAction::Ps), None);
    assert_eq!(control_cmd_string(&SlashAction::Stop), None);
}
