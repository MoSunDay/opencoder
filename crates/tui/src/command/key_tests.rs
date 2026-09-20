use super::*;

#[test]
fn tab_fills_input_with_command_name() {
    let mut menu = Some(CommandMenu::new());
    // Filter to /plan
    for c in "plan".chars() {
        if let Some(m) = menu.as_mut() {
            m.on_char(c);
        }
    }
    let (outcome, _quit) = handle_command_key(&mut menu, key(KeyCode::Tab, KeyModifiers::NONE));
    match outcome {
        CommandOutcome::FillInput(s) => assert_eq!(s, "/plan"),
        other => panic!("expected FillInput, got {:?}", other),
    }
    assert!(menu.is_none(), "popup closed after Tab-fill");
}

#[test]
fn space_fills_selected_command_for_compound_input() {
    let mut menu = Some(CommandMenu::new());
    for c in "plan".chars() {
        menu.as_mut().expect("menu open").on_char(c);
    }

    let (outcome, quit) =
        handle_command_key(&mut menu, key(KeyCode::Char(' '), KeyModifiers::NONE));

    assert!(!quit);
    assert!(matches!(outcome, CommandOutcome::FillInput(ref s) if s == "/plan"));
    assert!(menu.is_none(), "popup must close after Space-fill");
}

#[test]
fn space_with_no_matching_command_keeps_popup_open() {
    let mut menu = Some(CommandMenu::new());
    menu.as_mut().expect("menu open").paste("no-such-command");

    let (outcome, quit) =
        handle_command_key(&mut menu, key(KeyCode::Char(' '), KeyModifiers::NONE));

    assert!(!quit);
    assert!(matches!(outcome, CommandOutcome::Idle));
    assert_eq!(
        menu.as_ref().expect("popup stays open").query(),
        "no-such-command"
    );
}

#[test]
fn tab_on_non_control_command_fills_input() {
    let mut menu = Some(CommandMenu::new());
    // Filter to /task (non-control)
    for c in "task".chars() {
        if let Some(m) = menu.as_mut() {
            m.on_char(c);
        }
    }
    let (outcome, _quit) = handle_command_key(&mut menu, key(KeyCode::Tab, KeyModifiers::NONE));
    match outcome {
        CommandOutcome::FillInput(s) => assert_eq!(s, "/task"),
        other => panic!("expected FillInput, got {:?}", other),
    }
    assert!(menu.is_none(), "popup closed after Tab-fill");
}

#[test]
fn enter_on_control_command_dispatches() {
    let mut menu = Some(CommandMenu::new());
    // /act must dispatch directly even though /compact also contains "act".
    for c in "act".chars() {
        if let Some(m) = menu.as_mut() {
            m.on_char(c);
        }
    }
    let (outcome, _quit) = handle_command_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
    match outcome {
        CommandOutcome::Dispatch(SlashAction::Act) => {}
        other => panic!("expected Dispatch(Act), got {:?}", other),
    }
    assert!(menu.is_none(), "popup closed after Enter-dispatch");
}

#[test]
fn enter_on_clear_context_dispatches() {
    let mut menu = Some(CommandMenu::new());
    for c in "clear_context".chars() {
        if let Some(m) = menu.as_mut() {
            m.on_char(c);
        }
    }
    let (outcome, _quit) = handle_command_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
    match outcome {
        CommandOutcome::Dispatch(SlashAction::ClearContext) => {}
        other => panic!("expected Dispatch(ClearContext), got {:?}", other),
    }
}

#[test]
fn parse_local_commands() {
    assert_eq!(parse("/ps"), Some(SlashAction::Ps));
    assert_eq!(parse("/stop"), Some(SlashAction::Stop));
    assert_eq!(parse("/ap"), Some(SlashAction::Ap));
    assert_eq!(parse(" /ps "), Some(SlashAction::Ps));
    assert_eq!(parse(" /ap "), Some(SlashAction::Ap));
}

#[test]
fn enter_on_ps_dispatches() {
    let mut menu = Some(CommandMenu::new());
    for c in "ps".chars() {
        if let Some(m) = menu.as_mut() {
            m.on_char(c);
        }
    }
    let (outcome, _quit) = handle_command_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
    match outcome {
        CommandOutcome::Dispatch(SlashAction::Ps) => {}
        other => panic!("expected Dispatch(Ps), got {:?}", other),
    }
    assert!(menu.is_none(), "popup closed after Enter-dispatch");
}

#[test]
fn enter_on_stop_dispatches() {
    let mut menu = Some(CommandMenu::new());
    for c in "stop".chars() {
        if let Some(m) = menu.as_mut() {
            m.on_char(c);
        }
    }
    let (outcome, _quit) = handle_command_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
    match outcome {
        CommandOutcome::Dispatch(SlashAction::Stop) => {}
        other => panic!("expected Dispatch(Stop), got {:?}", other),
    }
}

#[test]
fn enter_on_ap_dispatches() {
    let mut menu = Some(CommandMenu::new());
    for c in "ap".chars() {
        if let Some(m) = menu.as_mut() {
            m.on_char(c);
        }
    }
    // Query "ap" also matches "/config" (its description contains
    // "api_key"), but a name hit ranks first — "/ap" is already the
    // highlighted row; moving down reaches the description hit.
    let (outcome, _quit) = handle_command_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
    match outcome {
        CommandOutcome::Dispatch(SlashAction::Ap) => {}
        other => panic!("expected Dispatch(Ap), got {:?}", other),
    }
    assert!(menu.is_none(), "popup closed after Enter-dispatch");

    let mut menu = Some(CommandMenu::new());
    for c in "ap".chars() {
        if let Some(m) = menu.as_mut() {
            m.on_char(c);
        }
    }
    menu.as_mut().expect("menu open").move_down();
    let (outcome, _quit) = handle_command_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
    match outcome {
        CommandOutcome::Dispatch(SlashAction::Config) => {}
        other => panic!("expected Dispatch(Config) after move_down, got {:?}", other),
    }
}

#[test]
fn tab_on_local_command_fills_input() {
    let mut menu = Some(CommandMenu::new());
    for c in "ps".chars() {
        if let Some(m) = menu.as_mut() {
            m.on_char(c);
        }
    }
    let (outcome, _quit) = handle_command_key(&mut menu, key(KeyCode::Tab, KeyModifiers::NONE));
    match outcome {
        CommandOutcome::FillInput(s) => assert_eq!(s, "/ps"),
        other => panic!("expected FillInput, got {:?}", other),
    }
    assert!(menu.is_none(), "popup closed after Tab-fill");
}

#[test]
fn parse_fork() {
    assert_eq!(parse("/fork"), Some(SlashAction::Fork));
    assert_eq!(parse("/fk"), Some(SlashAction::Fork)); // alias
    assert_eq!(parse("fork"), None); // bare name (no slash) -> None
    assert_eq!(parse(" /fork "), Some(SlashAction::Fork)); // trimmed
}

#[test]
fn dispatch_fork() {
    assert_eq!(dispatch("/fork"), Some(SlashAction::Fork));
    assert_eq!(dispatch("/fk"), None); // alias resolved by parse, not dispatch
}

#[test]
fn enter_on_fork_dispatches() {
    let mut menu = Some(CommandMenu::new());
    for c in "fork".chars() {
        if let Some(m) = menu.as_mut() {
            m.on_char(c);
        }
    }
    let (outcome, _quit) = handle_command_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
    match outcome {
        CommandOutcome::Dispatch(SlashAction::Fork) => {}
        other => panic!("expected Dispatch(Fork), got {:?}", other),
    }
    assert!(menu.is_none(), "popup closed after Enter-dispatch");
}

#[test]
fn short_key_command_removed() {
    assert_eq!(parse("/short_key"), None);
    // `/sk` is now the alias of `/skill` (default-injection toggles).
    assert_eq!(parse("/sk"), Some(SlashAction::Skill));
    assert_eq!(parse("short_key"), None);
    assert_eq!(dispatch("/short_key"), None);
}

#[test]
fn parse_annotation_full() {
    assert_eq!(parse("/annotation"), Some(SlashAction::Annotation));
}

#[test]
fn parse_annotation_alias() {
    assert_eq!(parse("/ann"), Some(SlashAction::Annotation));
}

#[test]
fn dispatch_annotation() {
    assert_eq!(dispatch("/annotation"), Some(SlashAction::Annotation));
}

#[test]
fn parse_notepad_full() {
    assert_eq!(parse("/notepad"), Some(SlashAction::Notepad));
}

#[test]
fn parse_notepad_alias() {
    assert_eq!(parse("/note"), Some(SlashAction::Notepad));
}

#[test]
fn dispatch_notepad() {
    assert_eq!(dispatch("/notepad"), Some(SlashAction::Notepad));
}

#[test]
fn parse_sidecar() {
    assert_eq!(parse("/sidecar"), Some(SlashAction::Sidecar));
    // The free-text composer intercept (`parse_sidecar_question`) claims
    // `/sidecar <question>` before Submit, so `parse` only ever sees the
    // bare token — anything after it is NOT a popup command.
    assert_eq!(parse("/sidecar extra"), None);
}

#[test]
fn dispatch_sidecar() {
    assert_eq!(dispatch("/sidecar"), Some(SlashAction::Sidecar));
    assert!(COMMANDS.iter().any(|(name, _)| *name == "/sidecar"));
}

#[test]
fn parse_mcp_full() {
    assert_eq!(parse("/mcp"), Some(SlashAction::Mcp));
}

#[test]
fn parse_mcp_alias() {
    assert_eq!(parse("/mc"), Some(SlashAction::Mcp));
}

#[test]
fn parse_model_and_alias() {
    assert_eq!(parse("/model"), Some(SlashAction::Model));
    assert_eq!(parse("/mdl"), Some(SlashAction::Model));
}

#[test]
fn dispatch_mcp() {
    assert_eq!(dispatch("/mcp"), Some(SlashAction::Mcp));
}

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, mods)
}
