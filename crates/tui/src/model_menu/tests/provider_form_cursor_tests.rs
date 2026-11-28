//! `ProviderForm` cursor placement inside the rendered `/model` popup.

use super::common::provider_cfg;
use crate::model_menu::provider_form::{ProviderField, ProviderForm};
use crate::model_menu::render_model_popup;
use crate::model_menu::state::ModelMenu;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;

// ── provider form cursor placement ────────────────────────────────────────

#[test]
fn provider_form_cursor_on_model_id() {
    let mut form = ProviderForm::new_blank(&provider_cfg());
    form.focus = ProviderField::ModelId;
    form.model_id = "glm-5.2".into();
    form.model_id_cursor = form.model_id.chars().count(); // end
    let menu = ModelMenu::Form(form);

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_model_popup(f, Rect::new(0, 0, 80, 24), 23, &menu);
        })
        .unwrap();

    // popup: x=4, y=10 (want_h = 12 + max(1 header row) = 13); model_id is
    // row 1; "glm-5.2" has 7 chars → cx = 4+1+15+7 = 27, cy = 10+1+1 = 12
    terminal.backend_mut().assert_cursor_position((27, 12));
}

#[test]
fn provider_form_cursor_on_api_key_uses_raw_buffer() {
    let mut form = ProviderForm::from_existing("svc", "u", "m", "orig-key-12345", vec![]);
    form.focus = ProviderField::ApiKey;
    form.api_key_edited = true;
    form.api_key_input = "n".into();
    form.api_key_cursor = form.api_key_input.chars().count(); // end
    let menu = ModelMenu::Form(form);

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_model_popup(f, Rect::new(0, 0, 80, 24), 23, &menu);
        })
        .unwrap();

    // api_key is row 3; raw buffer "n" (1 char) → cx = 4+1+15+1 = 21,
    // cy = 10+1+3 = 14
    terminal.backend_mut().assert_cursor_position((21, 14));
}

#[test]
fn provider_form_cursor_hidden_on_save_and_readonly_name() {
    // Save button: no text field → no cursor.
    let mut form = ProviderForm::new_blank(&provider_cfg());
    form.focus = ProviderField::Save;
    let menu = ModelMenu::Form(form);
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_model_popup(f, Rect::new(0, 0, 80, 24), 23, &menu);
        })
        .unwrap();
    terminal.backend_mut().assert_cursor_position((0, 0));

    // Read-only name in edit mode: no cursor either.
    let mut form = ProviderForm::from_existing("svc", "u", "m", "k", vec![]);
    form.focus = ProviderField::Name;
    let menu = ModelMenu::Form(form);
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_model_popup(f, Rect::new(0, 0, 80, 24), 23, &menu);
        })
        .unwrap();
    terminal.backend_mut().assert_cursor_position((0, 0));
}

#[test]
fn provider_form_cursor_inside_headers_cell() {
    let mut form = ProviderForm::new_blank(&provider_cfg());
    form.focus = ProviderField::Headers;
    form.headers_active = true;
    form.headers = crate::model_menu::headers::HeadersEditor::new(vec![
        ("X-Region".into(), "eu".into()),
        ("X-Env".into(), "prod".into()),
    ]);
    form.headers.selected = 1;
    form.headers.editing_value = false; // editing name of pair 1
    let menu = ModelMenu::Form(form);

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_model_popup(f, Rect::new(0, 0, 80, 24), 23, &menu);
        })
        .unwrap();

    // 2 headers → want_h = 13 → popup y = 23-13 = 10; pair rows start at
    // popup line 5; pair 1 (index 1) → cy = 10+1+5+1 = 17.
    // name column starts at popup col 5; "X-Env" has 5 chars → cx = 4+1+5+5 = 15
    terminal.backend_mut().assert_cursor_position((15, 17));
}

#[test]
fn provider_form_cursor_inside_headers_value() {
    let mut form = ProviderForm::new_blank(&provider_cfg());
    form.focus = ProviderField::Headers;
    form.headers_active = true;
    form.headers =
        crate::model_menu::headers::HeadersEditor::new(vec![("X-Region".into(), "eu-west".into())]);
    form.headers.editing_value = true; // editing value of pair 0
    let menu = ModelMenu::Form(form);

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_model_popup(f, Rect::new(0, 0, 80, 24), 23, &menu);
        })
        .unwrap();

    // pair 0 → cy = 11+1+5+0 = 17. value column starts at popup col 28
    // (5 spaces + 20-char name + " = "); "eu-west" has 7 chars → cx = 4+1+28+7 = 40
    terminal.backend_mut().assert_cursor_position((40, 17));
}
