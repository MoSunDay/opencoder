use super::*;

// ---- parse_key_spec ----

#[test]
fn parse_ctrl_letter() {
    let c = parse_key_spec("ctrl+h").unwrap();
    assert_eq!(c.mods, KeyModifiers::CONTROL);
    assert_eq!(c.code, KeyCode::Char('h'));
}

#[test]
fn parse_alt_letter() {
    let c = parse_key_spec("alt+f").unwrap();
    assert_eq!(c.mods, KeyModifiers::ALT);
    assert_eq!(c.code, KeyCode::Char('f'));
}

#[test]
fn parse_ctrl_shift_tab_normalizes_to_backtab() {
    let c = parse_key_spec("ctrl+shift+tab").unwrap();
    assert_eq!(c.mods, KeyModifiers::CONTROL);
    assert_eq!(c.code, KeyCode::BackTab);
}

#[test]
fn parse_alt_tab() {
    let c = parse_key_spec("alt+tab").unwrap();
    assert_eq!(c.mods, KeyModifiers::ALT);
    assert_eq!(c.code, KeyCode::Tab);
}

#[test]
fn parse_function_key() {
    let c = parse_key_spec("f1").unwrap();
    assert_eq!(c.mods, KeyModifiers::NONE);
    assert_eq!(c.code, KeyCode::F(1));
}

#[test]
fn parse_bare_letter() {
    let c = parse_key_spec("x").unwrap();
    assert_eq!(c.mods, KeyModifiers::NONE);
    assert_eq!(c.code, KeyCode::Char('x'));
}

#[test]
fn parse_whitespace_tolerant() {
    let c = parse_key_spec("  ctrl + h  ").unwrap();
    assert_eq!(c.mods, KeyModifiers::CONTROL);
    assert_eq!(c.code, KeyCode::Char('h'));
}

#[test]
fn parse_garbage_returns_none() {
    assert!(parse_key_spec("").is_none());
    assert!(parse_key_spec("ctrl+").is_none());
    assert!(parse_key_spec("+++").is_none());
}

// ---- KeyCombo::matches ----

fn ev(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, mods)
}

#[test]
fn match_exact() {
    let c = parse_key_spec("ctrl+h").unwrap();
    assert!(c.matches(&ev(KeyCode::Char('h'), KeyModifiers::CONTROL)));
}

#[test]
fn match_raw_control_char_with_flag() {
    // Ctrl+D under kitty protocol: raw \u{4} + CONTROL flag
    let c = parse_key_spec("ctrl+d").unwrap();
    assert!(c.matches(&ev(KeyCode::Char('\u{4}'), KeyModifiers::CONTROL)));
}

#[test]
fn match_raw_control_char_without_flag() {
    // Some terminals send Ctrl+D as raw \u{4} without CONTROL
    let c = parse_key_spec("ctrl+d").unwrap();
    assert!(c.matches(&ev(KeyCode::Char('\u{4}'), KeyModifiers::NONE)));
}

#[test]
fn match_case_insensitive_alt() {
    // Alt+F should match both lowercase 'f' and uppercase 'F'
    let c = parse_key_spec("alt+f").unwrap();
    assert!(c.matches(&ev(KeyCode::Char('f'), KeyModifiers::ALT)));
    assert!(c.matches(&ev(
        KeyCode::Char('F'),
        KeyModifiers::ALT | KeyModifiers::SHIFT
    )));
}

#[test]
fn match_tab_backtab_alt_tab() {
    // alt+tab matches both Tab and BackTab
    let c = parse_key_spec("alt+tab").unwrap();
    assert!(c.matches(&ev(KeyCode::Tab, KeyModifiers::ALT)));
    assert!(c.matches(&ev(KeyCode::BackTab, KeyModifiers::ALT)));
}

#[test]
fn match_ctrl_shift_tab() {
    let c = parse_key_spec("ctrl+shift+tab").unwrap();
    // BackTab + CONTROL
    assert!(c.matches(&ev(KeyCode::BackTab, KeyModifiers::CONTROL)));
    // Tab + CONTROL + SHIFT
    assert!(c.matches(&ev(
        KeyCode::Tab,
        KeyModifiers::CONTROL | KeyModifiers::SHIFT
    )));
}

#[test]
fn no_match_different_modifiers() {
    let c = parse_key_spec("ctrl+h").unwrap();
    assert!(!c.matches(&ev(KeyCode::Char('h'), KeyModifiers::ALT)));
    assert!(!c.matches(&ev(KeyCode::Char('h'), KeyModifiers::NONE)));
}

// ---- key_event_to_spec round-trip ----

#[test]
fn roundtrip_ctrl_letter() {
    let spec = "ctrl+h";
    let combo = parse_key_spec(spec).unwrap();
    // Simulate the event that matches this combo
    let event = ev(KeyCode::Char('h'), KeyModifiers::CONTROL);
    let back = key_event_to_spec(event).unwrap();
    let back_combo = parse_key_spec(&back).unwrap();
    assert!(back_combo.matches(&event));
    assert!(combo.matches(&event));
}

#[test]
fn roundtrip_raw_control_char() {
    // Raw \u{4} (Ctrl+D) → should produce "ctrl+d"
    let event = ev(KeyCode::Char('\u{4}'), KeyModifiers::CONTROL);
    let spec = key_event_to_spec(event).unwrap();
    assert_eq!(spec, "ctrl+d");
}

#[test]
fn roundtrip_function_key() {
    let event = ev(KeyCode::F(3), KeyModifiers::NONE);
    let spec = key_event_to_spec(event).unwrap();
    assert_eq!(spec, "f3");
}

// ---- KeyBindings::from_config ----

#[test]
fn from_config_uses_defaults() {
    let config = Config::default();
    let b = KeyBindings::from_config(&config);
    assert!(b
        .quit
        .matches(&ev(KeyCode::Char('d'), KeyModifiers::CONTROL)));
    assert!(b
        .help
        .matches(&ev(KeyCode::Char('h'), KeyModifiers::CONTROL)));
    assert!(b
        .switch_mode
        .matches(&ev(KeyCode::Char('t'), KeyModifiers::CONTROL)));
}

#[test]
fn from_config_respects_custom_spec() {
    let mut config = Config::default();
    config.keymap.set("help", "f1".into());
    let b = KeyBindings::from_config(&config);
    // F1 now triggers help
    assert!(b.help.matches(&ev(KeyCode::F(1), KeyModifiers::NONE)));
    // Old binding no longer triggers
    assert!(!b
        .help
        .matches(&ev(KeyCode::Char('h'), KeyModifiers::CONTROL)));
}

#[test]
fn from_config_falls_back_on_bad_spec() {
    let mut config = Config::default();
    config.keymap.set("help", "garbage!!!".into());
    let b = KeyBindings::from_config(&config);
    // Falls back to default ctrl+h
    assert!(b
        .help
        .matches(&ev(KeyCode::Char('h'), KeyModifiers::CONTROL)));
}

#[test]
fn from_config_respects_custom_mode_switch_spec() {
    let mut config = Config::default();
    config.keymap.set("switch_mode", "f2".into());
    let b = KeyBindings::from_config(&config);
    assert!(b
        .switch_mode
        .matches(&ev(KeyCode::F(2), KeyModifiers::NONE)));
    assert!(!b
        .switch_mode
        .matches(&ev(KeyCode::Char('t'), KeyModifiers::CONTROL)));
}
