//! Cursor-editing tests for the numeric config fields and provider form fields.
//!
//! Split into its own module so `config_tests.rs` / `provider_tests.rs` stay
//! under the per-file line limit while keeping each domain's tests together.

use super::common::{backspace, cfg, ctrl, key, left, provider_cfg, right};
use crate::model_menu::config_form::{ConfigField, ConfigForm};
use crate::model_menu::provider_form::{ProviderField, ProviderForm};
use crate::model_menu::render_model_popup;
use crate::model_menu::state::{handle_model_key, ModelMenu};
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;

// ── ConfigForm cursor editing ─────────────────────────────────────────────

#[test]
fn typing_digit_inserts_at_cursor() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::Config(ConfigForm::new(&cfg())));
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Config(f) => f,
            _ => unreachable!(),
        };
        f.focus = ConfigField::Threshold;
        f.threshold_input = "124".into();
        f.threshold_cursor = 2;
    }
    handle_model_key(&mut slot, key('3'));
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f,
        _ => unreachable!(),
    };
    assert_eq!(
        f.threshold_input, "1234",
        "a digit inserts at the cursor, not at the end"
    );
    assert_eq!(
        f.threshold_cursor, 3,
        "cursor advances past the inserted digit"
    );
}

#[test]
fn left_right_moves_numeric_cursor_without_changing_value() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::Config(ConfigForm::new(&cfg())));
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Config(f) => f,
            _ => unreachable!(),
        };
        f.focus = ConfigField::Threshold;
        f.threshold_input = "1234".into();
        f.threshold_cursor = 2;
    }
    handle_model_key(&mut slot, left());
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f,
        _ => unreachable!(),
    };
    assert_eq!(f.threshold_cursor, 1, "Left moves the cursor back one");
    assert_eq!(f.threshold_input, "1234", "value must not change");

    handle_model_key(&mut slot, right());
    handle_model_key(&mut slot, right());
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f,
        _ => unreachable!(),
    };
    assert_eq!(f.threshold_cursor, 3, "Right moves the cursor forward");
    assert_eq!(f.threshold_input, "1234", "value must still not change");
}

#[test]
fn left_right_cursor_clamps_at_zero_and_len() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::Config(ConfigForm::new(&cfg())));
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Config(f) => f,
            _ => unreachable!(),
        };
        f.focus = ConfigField::Fps;
        f.fps_input = "12".into();
        f.fps_cursor = 0;
    }
    handle_model_key(&mut slot, left());
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f,
        _ => unreachable!(),
    };
    assert_eq!(f.fps_cursor, 0, "Left at the start clamps to 0");

    for _ in 0..6 {
        handle_model_key(&mut slot, right());
    }
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f,
        _ => unreachable!(),
    };
    assert_eq!(f.fps_cursor, 2, "Right past the end clamps to len");
    assert_eq!(f.fps_input, "12", "value unchanged while moving cursor");
}

#[test]
fn backspace_deletes_char_before_cursor() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::Config(ConfigForm::new(&cfg())));
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Config(f) => f,
            _ => unreachable!(),
        };
        f.focus = ConfigField::ContextSize;
        f.context_size_input = "1234".into();
        f.context_size_cursor = 3;
    }
    handle_model_key(&mut slot, backspace());
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f,
        _ => unreachable!(),
    };
    assert_eq!(
        f.context_size_input, "124",
        "Backspace deletes the char before the cursor"
    );
    assert_eq!(f.context_size_cursor, 2, "cursor moves back one");
}

#[test]
fn ctrl_clear_resets_cursor_to_zero() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::Config(ConfigForm::new(&cfg())));
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Config(f) => f,
            _ => unreachable!(),
        };
        f.focus = ConfigField::MaxTokens;
        f.max_tokens_input = "8192".into();
        f.max_tokens_cursor = 2;
    }
    handle_model_key(&mut slot, ctrl('l'));
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f,
        _ => unreachable!(),
    };
    assert!(f.max_tokens_input.is_empty(), "Ctrl+L clears the field");
    assert_eq!(f.max_tokens_cursor, 0, "cursor resets to the start");
}

#[test]
fn config_form_cursor_renders_at_edit_position() {
    let mut form = ConfigForm::new(&cfg());
    form.focus = ConfigField::MaxTokens;
    form.max_tokens_input = "8192".into();
    form.max_tokens_cursor = 2; // between "81" and "92"
    let menu = ModelMenu::Config(form);

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_model_popup(f, Rect::new(0, 0, 80, 24), 23, &menu);
        })
        .unwrap();

    // cx = 4 + 1(border) + 15(label) + 2 = 22, cy = 8 (composer_top 23 - want_h 15) + 1 + 2 = 11
    terminal.backend_mut().assert_cursor_position((22, 11));
}

// ── ProviderForm cursor editing ───────────────────────────────────────────

#[test]
fn provider_typing_inserts_at_cursor() {
    let mut slot: Option<ModelMenu> =
        Some(ModelMenu::Form(ProviderForm::new_blank(&provider_cfg())));
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Form(f) => f,
            _ => unreachable!(),
        };
        f.focus = ProviderField::BaseUrl;
        f.base_url = "https://a.com/v1".into();
        f.base_url_cursor = 13; // between "https://a.com" and "/v1"
    }
    handle_model_key(&mut slot, key('x'));
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Form(f) => f,
        _ => unreachable!(),
    };
    assert_eq!(
        f.base_url, "https://a.comx/v1",
        "char inserts at the cursor"
    );
    assert_eq!(
        f.base_url_cursor, 14,
        "cursor advances past the inserted char"
    );
}

#[test]
fn provider_left_right_moves_cursor_without_changing_value() {
    let mut slot: Option<ModelMenu> =
        Some(ModelMenu::Form(ProviderForm::new_blank(&provider_cfg())));
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Form(f) => f,
            _ => unreachable!(),
        };
        f.focus = ProviderField::ModelId;
        f.model_id = "glm-5.2".into();
        f.model_id_cursor = 2;
    }
    handle_model_key(&mut slot, left());
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Form(f) => f,
        _ => unreachable!(),
    };
    assert_eq!(f.model_id_cursor, 1, "Left moves the cursor back");
    assert_eq!(f.model_id, "glm-5.2", "value must not change");

    handle_model_key(&mut slot, right());
    handle_model_key(&mut slot, right());
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Form(f) => f,
        _ => unreachable!(),
    };
    assert_eq!(f.model_id_cursor, 3, "Right moves the cursor forward");
    assert_eq!(f.model_id, "glm-5.2", "value must still not change");
}

#[test]
fn provider_backspace_deletes_char_before_cursor() {
    let mut slot: Option<ModelMenu> =
        Some(ModelMenu::Form(ProviderForm::new_blank(&provider_cfg())));
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Form(f) => f,
            _ => unreachable!(),
        };
        f.focus = ProviderField::ModelId;
        f.model_id = "glm-5.2".into();
        f.model_id_cursor = 4; // after "glm-"
    }
    handle_model_key(&mut slot, backspace());
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Form(f) => f,
        _ => unreachable!(),
    };
    assert_eq!(
        f.model_id, "glm5.2",
        "Backspace deletes the char before the cursor"
    );
    assert_eq!(f.model_id_cursor, 3, "cursor moves back one");
}

#[test]
fn provider_cursor_renders_at_edit_position() {
    let mut form = ProviderForm::new_blank(&provider_cfg());
    form.focus = ProviderField::ModelId;
    form.model_id = "glm-5.2".into();
    form.model_id_cursor = 3; // after "glm"
    let menu = ModelMenu::Form(form);

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_model_popup(f, Rect::new(0, 0, 80, 24), 23, &menu);
        })
        .unwrap();

    // popup: x=4, y=11; model_id is row 1; cursor at col 3 → cx = 4+1+15+3 = 23
    terminal.backend_mut().assert_cursor_position((23, 13));
}
