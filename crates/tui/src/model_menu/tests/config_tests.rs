//! Tests for ConfigPatch serialization and ConfigForm key handling.

use super::common::{backspace, cfg, ctrl, enter, key, left, right};
use crate::model_menu::config_form::{ConfigField, ConfigForm, Reasoning};
use crate::model_menu::patch::ConfigPatch;
use crate::model_menu::render_model_popup;
use crate::model_menu::state::{handle_model_key, ModelMenu, ModelOutcome};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;

// ── ConfigPatch ───────────────────────────────────────────────────────────

#[test]
fn config_patch_serializes_all_fields() {
    let p = ConfigPatch {
        reasoning_effort: Some("high".into()),
        interleaved_thinking: Some(true),
        max_tokens: Some(8192),
        context_threshold: 80_000,
        context_limit: 128_000,
        fps: 25,
        ap_max_iter: 15,
        theme: "dark".into(),
        enable_tmux_session: None,
    };
    let v = p.to_json();
    assert_eq!(v["reasoning_effort"], serde_json::json!("high"));
    assert_eq!(v["interleaved_thinking"], serde_json::json!(true));
    assert_eq!(v["max_tokens"], serde_json::json!(8192));
    assert_eq!(v["fps"], serde_json::json!(25));
    assert_eq!(
        v["compaction"]["context_threshold"],
        serde_json::json!(80_000)
    );
    assert_eq!(v["context_limit"], serde_json::json!(128_000));
    assert_eq!(v["autopilot"]["max_iterations"], serde_json::json!(15));
    assert_eq!(v["theme"], serde_json::json!("dark"));
    assert_eq!(v["enable_tmux_session"], serde_json::json!(null));
}

#[test]
fn config_patch_omits_max_tokens_when_none() {
    let p = ConfigPatch {
        reasoning_effort: None,
        interleaved_thinking: None,
        max_tokens: None,
        context_threshold: 1000,
        context_limit: 128_000,
        fps: 10,
        ap_max_iter: 10,
        theme: "dark".into(),
        enable_tmux_session: None,
    };
    let v = p.to_json();
    assert!(
        v.get("max_tokens").is_none(),
        "max_tokens must be absent when None"
    );
}

// ── ConfigForm ────────────────────────────────────────────────────────────

#[test]
fn config_form_defaults_fps_to_ten() {
    let f = ConfigForm::new(&cfg());
    assert_eq!(f.fps_input, "10");
    assert_eq!(f.build_patch().fps, 10);
}

#[test]
fn enter_chains_through_config_fields_to_save() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::Config(ConfigForm::new(&cfg())));
    let order = [
        ConfigField::InterleavedThinking,
        ConfigField::MaxTokens,
        ConfigField::ContextSize,
        ConfigField::Threshold,
        ConfigField::Fps,
        ConfigField::ApMaxIter,
        ConfigField::Theme,
        ConfigField::EnableTmuxSession,
        ConfigField::Save,
    ];
    for expect in &order {
        handle_model_key(&mut slot, enter());
        let f = match slot.as_ref() {
            Some(ModelMenu::Config(f)) => f,
            _ => panic!("menu should stay Config until Save"),
        };
        assert_eq!(&f.focus, expect, "Enter should advance to next field");
    }
    // One more Enter on Save → Save outcome, menu closes.
    let outcome = handle_model_key(&mut slot, enter());
    assert!(matches!(outcome, ModelOutcome::Save(_)));
    assert!(slot.is_none(), "slot cleared after Save");
}

#[test]
fn left_right_change_reasoning() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::Config(ConfigForm::new(&cfg())));
    let before = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f.reasoning,
        _ => unreachable!(),
    };
    handle_model_key(&mut slot, right());
    let after = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f.reasoning,
        _ => unreachable!(),
    };
    assert_eq!(after, before.next(), "Right advances reasoning");
    handle_model_key(&mut slot, left());
    let back = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f.reasoning,
        _ => unreachable!(),
    };
    assert_eq!(back, before, "Left returns reasoning to original");
}

#[test]
fn left_right_toggle_interleave() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::Config(ConfigForm::new(&cfg())));
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Config(f) => f,
            _ => unreachable!(),
        };
        f.focus = ConfigField::InterleavedThinking;
    }
    let before = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f.interleaved_thinking,
        _ => unreachable!(),
    };
    handle_model_key(&mut slot, right());
    let after = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f.interleaved_thinking,
        _ => unreachable!(),
    };
    assert_eq!(after, !before, "Right toggles interleave");
}

#[test]
fn config_form_theme_cycles_with_space() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::Config(ConfigForm::new(&cfg())));
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Config(f) => f,
            _ => unreachable!(),
        };
        assert_eq!(f.theme, crate::theme::ThemeKind::Dark);
        f.focus = ConfigField::Theme;
    }
    handle_model_key(&mut slot, key(' '));
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f,
        _ => unreachable!(),
    };
    assert_eq!(f.theme, crate::theme::ThemeKind::Light);
}

#[test]
fn typing_digits_sets_fps() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::Config(ConfigForm::new(&cfg())));
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Config(f) => f,
            _ => unreachable!(),
        };
        f.focus = ConfigField::Fps;
        f.fps_input = "2".into();
    }
    handle_model_key(&mut slot, key('4'));
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f,
        _ => unreachable!(),
    };
    assert_eq!(f.fps_input, "24", "from \"2\", typing '4' yields \"24\"");
}

#[test]
fn typing_digits_sets_max_tokens() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::Config(ConfigForm::new(&cfg())));
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Config(f) => f,
            _ => unreachable!(),
        };
        f.focus = ConfigField::MaxTokens;
    }
    for c in "8192".chars() {
        handle_model_key(&mut slot, key(c));
    }
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f,
        _ => unreachable!(),
    };
    assert_eq!(f.max_tokens_input, "8192");
    assert_eq!(f.build_patch().max_tokens, Some(8192));
}

// ── paste routing (ConfigForm) ───────────────────────────────────────────

#[test]
fn config_form_paste_into_max_tokens() {
    let mut f = ConfigForm::new(&cfg());
    f.focus = ConfigField::MaxTokens;
    f.paste_into("8192");
    assert_eq!(f.max_tokens_input, "8192");
    assert_eq!(f.build_patch().max_tokens, Some(8192));
}

#[test]
fn config_form_paste_filters_non_digits_in_max_tokens() {
    let mut f = ConfigForm::new(&cfg());
    f.focus = ConfigField::MaxTokens;
    f.paste_into("12abc3");
    assert_eq!(f.max_tokens_input, "123", "only ascii digits are kept");
}

#[test]
fn config_form_paste_into_fps_clamps_at_30() {
    let mut f = ConfigForm::new(&cfg());
    f.focus = ConfigField::Fps;
    f.fps_input = "2".into();
    f.paste_into("4");
    assert_eq!(f.fps_input, "24", "2 -> append 4 -> 24");
    f.fps_input = "2".into();
    f.paste_into("99");
    assert_eq!(f.fps_input, "299", "paste appends without clamp");
    assert_eq!(f.build_patch().fps, 30, "clamped to 30 at build time");
}

#[test]
fn config_form_paste_into_threshold() {
    let mut f = ConfigForm::new(&cfg());
    f.focus = ConfigField::Threshold;
    f.threshold_input = "1000".into();
    f.paste_into("000");
    assert_eq!(
        f.threshold_input, "1000000",
        "1000 -> append 000 -> 1000000"
    );
}

#[test]
fn config_form_inits_autopilot_from_config() {
    let mut c = cfg();
    c.autopilot.enabled = true;
    c.autopilot.max_iterations = 7;
    let f = ConfigForm::new(&c);
    assert_eq!(f.ap_max_iter_input, "7");
    let p = f.build_patch();
    assert_eq!(p.ap_max_iter, 7);
}

#[test]
fn config_form_inits_context_size_from_config() {
    let f = ConfigForm::new(&cfg());
    assert_eq!(
        f.context_size_input, "128000",
        "default context size is 128k"
    );

    let mut c = cfg();
    c.context_limit = Some(200_000);
    let f = ConfigForm::new(&c);
    assert_eq!(f.context_size_input, "200000");
}

#[test]
fn typing_digits_sets_context_size() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::Config(ConfigForm::new(&cfg())));
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Config(f) => f,
            _ => unreachable!(),
        };
        f.focus = ConfigField::ContextSize;
        f.context_size_input = "0".into();
    }
    handle_model_key(&mut slot, key('2'));
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f,
        _ => unreachable!(),
    };
    assert_eq!(f.context_size_input, "02");
    assert_eq!(f.build_patch().context_limit, 2);
}

#[test]
fn backspace_pops_digit_from_threshold() {
    let mut f = ConfigForm::new(&cfg());
    f.focus = ConfigField::Threshold;
    f.threshold_input = "50000".into();
    let (_outcome, next) = crate::model_menu::config_form::handle_key(f, backspace());
    let f = match next {
        Some(ModelMenu::Config(f)) => f,
        _ => panic!("expected Config menu"),
    };
    assert_eq!(f.threshold_input, "5000", "\"50000\" pop -> \"5000\"");
}

#[test]
fn backspace_pops_digit_from_context_size() {
    let mut f = ConfigForm::new(&cfg());
    f.focus = ConfigField::ContextSize;
    f.context_size_input = "50000".into();
    let (_outcome, next) = crate::model_menu::config_form::handle_key(f, backspace());
    let f = match next {
        Some(ModelMenu::Config(f)) => f,
        _ => panic!("expected Config menu"),
    };
    assert_eq!(f.context_size_input, "5000", "\"50000\" pop -> \"5000\"");
}

#[test]
fn validate_rejects_threshold_above_context_size() {
    let mut f = ConfigForm::new(&cfg());
    f.threshold_input = "200000".into();
    f.context_size_input = "128000".into();
    // validate is private, so trigger it via handle_key on Save.
    f.focus = ConfigField::Save;
    let (outcome, next) = crate::model_menu::config_form::handle_key(f, enter());
    // Should NOT save; should stay as Config with an error.
    assert!(
        matches!(outcome, ModelOutcome::Idle),
        "save should be blocked"
    );
    assert!(next.is_some(), "menu should stay open on validation error");
}

#[test]
fn config_patch_writes_context_limit() {
    let mut f = ConfigForm::new(&cfg());
    f.context_size_input = "96000".into();
    let v = f.build_patch().to_json();
    assert_eq!(v["context_limit"], serde_json::json!(96_000));
}

// ── String-buffer numeric fields (Issue 2: clearable inputs) ───────────────

#[test]
fn backspace_clears_threshold_to_empty() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::Config(ConfigForm::new(&cfg())));
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Config(f) => f,
            _ => unreachable!(),
        };
        f.focus = ConfigField::Threshold;
        f.threshold_input = "1000".into();
    }
    // Backspace 4 times should fully clear the field (no floor stuck).
    for _ in 0..4 {
        handle_model_key(&mut slot, backspace());
    }
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f,
        _ => unreachable!(),
    };
    assert_eq!(
        f.threshold_input, "",
        "backspace must clear threshold to empty"
    );
}

#[test]
fn type_digits_replaces_value() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::Config(ConfigForm::new(&cfg())));
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Config(f) => f,
            _ => unreachable!(),
        };
        f.focus = ConfigField::ContextSize;
        f.context_size_input = "999".into();
    }
    // Clear the old value, then type a fresh one.
    for _ in 0..3 {
        handle_model_key(&mut slot, backspace());
    }
    for c in "42".chars() {
        handle_model_key(&mut slot, key(c));
    }
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f,
        _ => unreachable!(),
    };
    assert_eq!(f.context_size_input, "42", "clear then type replaces value");
    assert_eq!(f.build_patch().context_limit, 42);
}

#[test]
fn save_empty_field_shows_error() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::Config(ConfigForm::new(&cfg())));
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Config(f) => f,
            _ => unreachable!(),
        };
        f.focus = ConfigField::Save;
        f.threshold_input = "".into();
    }
    let outcome = handle_model_key(&mut slot, enter());
    assert!(
        matches!(outcome, ModelOutcome::Idle),
        "empty field must block save"
    );
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f,
        _ => unreachable!(),
    };
    assert!(f.error.is_some(), "an error message should be shown");
}

// ── config form cursor placement ──────────────────────────────────────────

#[test]
fn config_form_cursor_on_max_tokens() {
    let mut form = ConfigForm::new(&cfg());
    form.focus = ConfigField::MaxTokens;
    form.max_tokens_input = "8192".into();
    form.max_tokens_cursor = form.max_tokens_input.chars().count(); // end
    let menu = ModelMenu::Config(form);

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_model_popup(f, Rect::new(0, 0, 80, 24), 23, &menu);
        })
        .unwrap();

    // popup: x=4, y=8 (composer_top 23 - want_h 15); max_tokens is row 2; "8192" has 4 chars
    // cx = 4 + 1(border) + 15(label) + 4 = 24, cy = 8 + 1(border) + 2 = 11
    terminal.backend_mut().assert_cursor_position((24, 11));
}

#[test]
fn config_form_cursor_on_context_size() {
    let mut form = ConfigForm::new(&cfg());
    form.focus = ConfigField::ContextSize;
    form.context_size_input = "128000".into();
    form.context_size_cursor = form.context_size_input.chars().count(); // end
    let menu = ModelMenu::Config(form);

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_model_popup(f, Rect::new(0, 0, 80, 24), 23, &menu);
        })
        .unwrap();

    // popup: x=4, y=8 (composer_top 23 - want_h 15); ctx size is row 3; "128000" has 6 chars
    // cx = 4 + 1 + 15 + 6 = 26, cy = 8 + 1 + 3 = 12
    // cursor sits at end of raw "128000", before decorative " tokens" suffix
    terminal.backend_mut().assert_cursor_position((26, 12));
}

#[test]
fn config_form_cursor_hidden_on_toggle() {
    let mut form = ConfigForm::new(&cfg());
    form.focus = ConfigField::Reasoning;
    let menu = ModelMenu::Config(form);

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_model_popup(f, Rect::new(0, 0, 80, 24), 23, &menu);
        })
        .unwrap();

    // No set_cursor_position called for non-text fields
    // cursor stays at initial (0,0) and terminal hides it
    terminal.backend_mut().assert_cursor_position((0, 0));
}

#[test]
fn config_form_cursor_hidden_on_save_button() {
    let mut form = ConfigForm::new(&cfg());
    form.focus = ConfigField::Save;
    let menu = ModelMenu::Config(form);

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_model_popup(f, Rect::new(0, 0, 80, 24), 23, &menu);
        })
        .unwrap();

    terminal.backend_mut().assert_cursor_position((0, 0));
}

// ── numeric cursor editing (Left/Right, insert/delete at cursor) ─────────

// ── Ctrl+L / Ctrl+U clear focused field ───────────────────────────────────

#[test]
fn ctrl_u_clears_focused_numeric_field() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::Config(ConfigForm::new(&cfg())));
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Config(f) => f,
            _ => unreachable!(),
        };
        f.focus = ConfigField::MaxTokens;
        f.max_tokens_input = "8192".into();
    }
    handle_model_key(&mut slot, ctrl('u'));
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f,
        _ => unreachable!(),
    };
    assert!(
        f.max_tokens_input.is_empty(),
        "Ctrl+U must clear max_tokens"
    );
    assert_eq!(f.focus, ConfigField::MaxTokens, "focus must stay");
}

#[test]
fn ctrl_l_clears_focused_field_and_raw_control_char_forms_match() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::Config(ConfigForm::new(&cfg())));
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Config(f) => f,
            _ => unreachable!(),
        };
        f.focus = ConfigField::ContextSize;
        f.context_size_input = "128000".into();
    }
    // Raw control-char form (kitty keyboard protocol reports \u{c} for Ctrl+L).
    handle_model_key(
        &mut slot,
        crossterm::event::KeyEvent::new(KeyCode::Char('\u{c}'), KeyModifiers::CONTROL),
    );
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f,
        _ => unreachable!(),
    };
    assert!(
        f.context_size_input.is_empty(),
        "raw Ctrl+L must clear context_size"
    );

    // Ctrl+L char form clears fps; raw \u{15} clears ap_max_iter.
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Config(f) => f,
            _ => unreachable!(),
        };
        f.focus = ConfigField::Fps;
        f.fps_input = "24".into();
    }
    handle_model_key(&mut slot, ctrl('l'));
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f,
        _ => unreachable!(),
    };
    assert!(f.fps_input.is_empty(), "Ctrl+L must clear fps");

    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Config(f) => f,
            _ => unreachable!(),
        };
        f.focus = ConfigField::ApMaxIter;
        f.ap_max_iter_input = "9".into();
    }
    handle_model_key(
        &mut slot,
        crossterm::event::KeyEvent::new(KeyCode::Char('\u{15}'), KeyModifiers::CONTROL),
    );
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f,
        _ => unreachable!(),
    };
    assert!(
        f.ap_max_iter_input.is_empty(),
        "raw \u{15} (Ctrl+U) must clear ap_max_iter"
    );
}

#[test]
fn ctrl_clear_is_noop_on_toggle_and_button_fields() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::Config(ConfigForm::new(&cfg())));
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Config(f) => f,
            _ => unreachable!(),
        };
        f.focus = ConfigField::Reasoning;
        f.reasoning = Reasoning::High;
    }
    handle_model_key(&mut slot, ctrl('u'));
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f,
        _ => unreachable!(),
    };
    assert_eq!(
        f.reasoning,
        Reasoning::High,
        "Ctrl+U must not touch toggle fields"
    );

    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Config(f) => f,
            _ => unreachable!(),
        };
        f.focus = ConfigField::Save;
    }
    handle_model_key(&mut slot, ctrl('l'));
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f,
        _ => unreachable!(),
    };
    assert_eq!(f.focus, ConfigField::Save, "Ctrl+L must not move focus");
    assert!(slot.is_some(), "menu must stay open after Ctrl+L");
}

#[test]
fn ctrl_d_still_quits_in_config() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::Config(ConfigForm::new(&cfg())));
    let out = handle_model_key(&mut slot, ctrl('d'));
    assert!(matches!(out, ModelOutcome::Quit), "Ctrl+D must quit modal");
    assert!(slot.is_none());
}

// ── enable_tmux_session toggle ────────────────────────────────────────────

#[test]
fn config_form_tmux_defaults_off_when_config_none() {
    let form = ConfigForm::new(&cfg());
    assert!(
        !form.enable_tmux_session,
        "enable_tmux_session must default to false when Config field is None"
    );
}

#[test]
fn config_form_tmux_reads_true_from_config() {
    let mut c = cfg();
    c.enable_tmux_session = Some(true);
    let form = ConfigForm::new(&c);
    assert!(form.enable_tmux_session);
}

#[test]
fn config_form_tmux_toggles_on_left_right_and_space() {
    for k in [left(), right(), key(' ')] {
        let mut slot: Option<ModelMenu> = Some(ModelMenu::Config(ConfigForm::new(&cfg())));
        {
            let f = match slot.as_mut().unwrap() {
                ModelMenu::Config(f) => f,
                _ => unreachable!(),
            };
            f.focus = ConfigField::EnableTmuxSession;
        }
        handle_model_key(&mut slot, k);
        let f = match slot.as_ref().unwrap() {
            ModelMenu::Config(f) => f,
            _ => unreachable!(),
        };
        assert!(
            f.enable_tmux_session,
            "Left/Right/Space must flip the tmux toggle on"
        );
    }
}

#[test]
fn config_form_tmux_patch_serializes_true_when_on() {
    let mut form = ConfigForm::new(&cfg());
    form.enable_tmux_session = true;
    let v = form.build_patch().to_json();
    assert_eq!(v["enable_tmux_session"], serde_json::json!(true));
}
