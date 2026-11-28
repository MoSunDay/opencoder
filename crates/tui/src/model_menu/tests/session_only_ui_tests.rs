//! Real key-dispatch coverage for session-only Provider CRUD and secret/header edits.

use super::common::{enter, esc, key, provider_cfg};
use crate::model_menu::{
    handle_model_key, render_model_popup, ModelMenu, ModelOutcome, ProviderField, ProviderList,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};

fn key_code(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

fn arm_form_save(slot: &mut Option<ModelMenu>) {
    let Some(ModelMenu::Form(form)) = slot.as_mut() else {
        panic!("Provider form must be open")
    };
    form.focus = ProviderField::Save;
    assert!(matches!(
        handle_model_key(slot, enter()),
        ModelOutcome::Idle
    ));
    assert!(matches!(
        slot,
        Some(ModelMenu::Form(form)) if form.confirm_save_default.is_some()
    ));
}

#[test]
fn add_form_n_returns_session_only_patch_and_enter_preserves_disk_save() {
    for (confirmation, disk_save) in [(key('n'), false), (enter(), true)] {
        let mut slot = Some(ModelMenu::List(ProviderList::new(&provider_cfg())));
        handle_model_key(&mut slot, key('n'));
        let Some(ModelMenu::Form(form)) = slot.as_mut() else {
            panic!("n opens the add form")
        };
        form.name = "temporary".into();
        form.model_id = "session-model".into();
        form.base_url = "https://example.invalid/v1".into();
        arm_form_save(&mut slot);
        let outcome = handle_model_key(&mut slot, confirmation);
        let patch = match (disk_save, outcome) {
            (false, ModelOutcome::SaveSessionOnly(patch)) => patch,
            (true, ModelOutcome::Save(patch)) => patch,
            _ => panic!("confirmation must select the requested save scope"),
        };
        assert!(
            patch["model"].is_null() && patch.get("modes").is_none(),
            "a scoped provider add never selects a model: {patch}"
        );
        assert_eq!(
            patch["providers"]["temporary"]["base_url"],
            "https://example.invalid/v1"
        );
        assert!(slot.is_none());
    }
}

#[test]
fn edit_api_key_and_headers_reach_session_only_outcome_through_form_keys() {
    let mut slot = Some(ModelMenu::List(ProviderList::new(&provider_cfg())));
    handle_model_key(&mut slot, key('e'));
    let Some(ModelMenu::Form(form)) = slot.as_mut() else {
        panic!("e opens the edit form")
    };
    form.focus = ProviderField::ApiKey;
    handle_model_key(&mut slot, key('k'));
    let Some(ModelMenu::Form(form)) = slot.as_mut() else {
        unreachable!()
    };
    form.focus = ProviderField::Headers;
    handle_model_key(&mut slot, enter());
    handle_model_key(&mut slot, key('+'));
    handle_model_key(&mut slot, key('X'));
    handle_model_key(&mut slot, key_code(KeyCode::Right));
    handle_model_key(&mut slot, key('v'));
    handle_model_key(&mut slot, enter());
    arm_form_save(&mut slot);

    let ModelOutcome::SaveSessionOnly(patch) = handle_model_key(&mut slot, key('N')) else {
        panic!("N must apply the edited Provider only to this session")
    };
    assert_eq!(patch["providers"]["deepseek"]["api_key"], "k");
    assert!(patch["providers"]["deepseek"]["headers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|header| header["name"] == "X" && header["value"] == "v"));
}

#[test]
fn delete_n_is_session_only_while_y_saves_and_esc_cancels() {
    for (confirmation, expected) in [(key('n'), "session"), (key('y'), "disk"), (esc(), "cancel")] {
        let mut slot = Some(ModelMenu::List(ProviderList::new(&provider_cfg())));
        handle_model_key(&mut slot, key('d'));
        match (handle_model_key(&mut slot, confirmation), expected) {
            (ModelOutcome::SaveSessionOnly(patch), "session") => {
                assert_eq!(patch["providers"]["deepseek"], serde_json::Value::Null);
            }
            (ModelOutcome::Save(patch), "disk") => {
                assert_eq!(patch["providers"]["deepseek"], serde_json::Value::Null);
            }
            (ModelOutcome::Idle, "cancel") => {
                assert!(matches!(
                    slot,
                    Some(ModelMenu::List(list)) if list.confirm_delete.is_none()
                ));
            }
            _ => panic!("delete confirmation selected the wrong scope"),
        }
    }
}

#[test]
fn provider_form_confirmation_is_visible_with_session_only_hint() {
    let mut slot = Some(ModelMenu::List(ProviderList::new(&provider_cfg())));
    handle_model_key(&mut slot, key('e'));
    arm_form_save(&mut slot);
    let Some(menu) = slot else { unreachable!() };
    let backend = TestBackend::new(90, 26);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            let area = frame.area();
            render_model_popup(frame, area, 22, &menu);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let text = (0..26)
        .map(|y| {
            (0..90)
                .filter_map(|x| buffer.cell((x, y)).map(|cell| cell.symbol()))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Save Provider default?"), "{text}");
    assert!(text.contains("[n] session-only"), "{text}");
    assert!(text.contains("[y]/Enter save"), "{text}");
}
