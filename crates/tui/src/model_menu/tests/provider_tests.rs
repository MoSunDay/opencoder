//! `ProviderList` tests: construction, Enter/y/n flows, transitions to the
//! form, `model_id` fallback for providers without a model, and the
//! save-default confirmation dialog.

use super::common::{enter, esc, key, provider_cfg};
use crate::model_menu::list::ProviderList;
use crate::model_menu::provider_form::ProviderField;
use crate::model_menu::render_model_popup;
use crate::model_menu::state::{handle_model_key, ModelMenu, ModelOutcome};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

// ── ProviderList ──────────────────────────────────────────────────────────

#[test]
fn provider_list_builds_from_config() {
    let list = ProviderList::new(&provider_cfg());
    assert_eq!(list.entries.len(), 1);
    assert_eq!(list.entries[0].name, "deepseek");
    assert_eq!(list.entries[0].model_id, "deepseek-chat");
    assert_eq!(list.entries[0].headers.len(), 1);
    assert!(list.entries[0].active);
}

#[test]
fn list_enter_arms_save_default_prompt() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::List(ProviderList::new(&provider_cfg())));
    // Enter arms the "save as default?" prompt; it must NOT switch immediately.
    let outcome = handle_model_key(&mut slot, enter());
    assert!(
        matches!(outcome, ModelOutcome::Idle),
        "Enter should arm the prompt (Idle)"
    );
    assert!(slot.is_some(), "menu must stay open while prompting");
}

#[test]
fn list_enter_then_y_saves_as_default() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::List(ProviderList::new(&provider_cfg())));
    handle_model_key(&mut slot, enter());
    match handle_model_key(&mut slot, key('y')) {
        ModelOutcome::Save(json) => {
            assert_eq!(
                json["modes"]["act"]["model"],
                serde_json::json!("deepseek/deepseek-chat")
            );
            assert!(json["model"].is_null(), "no Shared default written");
        }
        _ => panic!("y should Save (persist default)"),
    }
    assert!(slot.is_none(), "menu closes after Save");
}

#[test]
fn list_enter_then_n_is_session_only() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::List(ProviderList::new(&provider_cfg())));
    handle_model_key(&mut slot, enter());
    match handle_model_key(&mut slot, key('n')) {
        ModelOutcome::SaveSessionOnly(json) => {
            assert_eq!(
                json["modes"]["act"]["model"],
                serde_json::json!("deepseek/deepseek-chat")
            );
            assert!(json["model"].is_null(), "no Shared default written");
        }
        _ => panic!("n should be SaveSessionOnly"),
    }
    assert!(slot.is_none(), "menu closes after session-only switch");
}

#[test]
fn list_enter_then_enter_saves_as_default() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::List(ProviderList::new(&provider_cfg())));
    handle_model_key(&mut slot, enter());
    match handle_model_key(&mut slot, enter()) {
        ModelOutcome::Save(json) => {
            assert_eq!(
                json["modes"]["act"]["model"],
                serde_json::json!("deepseek/deepseek-chat")
            );
        }
        _ => panic!("second Enter should Save (persist as global default)"),
    }
    assert!(slot.is_none(), "menu closes after Save");
}

#[test]
fn list_confirm_then_esc_cancels_without_switching() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::List(ProviderList::new(&provider_cfg())));
    handle_model_key(&mut slot, enter()); // arm "save as default?" prompt
    let outcome = handle_model_key(&mut slot, esc());
    assert!(
        matches!(outcome, ModelOutcome::Cancel),
        "Esc in confirm-save-default should Cancel",
    );
    assert!(slot.is_none(), "menu closes after Cancel");
}

#[test]
fn list_e_transitions_to_form() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::List(ProviderList::new(&provider_cfg())));
    handle_model_key(&mut slot, key('e'));
    assert!(
        matches!(slot, Some(ModelMenu::Form(_))),
        "e should transition to Form"
    );
    if let Some(ModelMenu::Form(f)) = &slot {
        assert!(f.name_readonly, "editing existing -> name is read-only");
        assert_eq!(f.name, "deepseek");
        assert_eq!(f.model_id, "deepseek-chat");
        assert_eq!(f.headers.pairs.len(), 1);
    }
}

#[test]
fn list_n_transitions_to_blank_form() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::List(ProviderList::new(&provider_cfg())));
    handle_model_key(&mut slot, key('n'));
    assert!(matches!(slot, Some(ModelMenu::Form(_))));
    if let Some(ModelMenu::Form(f)) = &slot {
        assert!(!f.name_readonly, "new provider -> name is editable");
        assert!(f.name.is_empty());
    }
}

#[test]
fn list_d_then_y_deletes() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::List(ProviderList::new(&provider_cfg())));
    handle_model_key(&mut slot, key('d'));
    assert!(matches!(&slot, Some(ModelMenu::List(l)) if l.confirm_delete.is_some()));
    match handle_model_key(&mut slot, key('y')) {
        ModelOutcome::Save(json) => {
            assert_eq!(json["providers"]["deepseek"], serde_json::Value::Null);
        }
        _ => panic!("y should delete (Save)"),
    }
}

// ── model_id fallback for unset provider model ────────────────────────────
//
// Regression: a provider without an explicit `model` field used to fall back
// to the active model's id for EVERY provider, producing wrong/misleading
// rows. Now only the active provider derives its model_id from the top-level
// config; non-active providers with no `model` field show as unset (empty).

/// Config with two providers, neither carrying an explicit `model` field.
/// Active model is "active/active-model".
fn multi_provider_cfg() -> opencoder_core::Config {
    let mut c = super::common::cfg();
    c.model = "active/active-model".to_string();
    c.providers.insert(
        "active".to_string(),
        opencoder_core::ProviderConfig {
            temperature: None,
            protocol: "chat_completions".into(),
            base_url: "https://active.example.com/v1".to_string(),
            api_key: Some("ak-active".to_string()),
            model: None,
            headers: Vec::new(),
        },
    );
    c.providers.insert(
        "other".to_string(),
        opencoder_core::ProviderConfig {
            temperature: None,
            protocol: "chat_completions".into(),
            base_url: "https://other.example.com/v1".to_string(),
            api_key: None,
            model: None,
            headers: Vec::new(),
        },
    );
    c
}

#[test]
fn provider_list_model_id_empty_for_non_active_without_model() {
    let list = ProviderList::new(&multi_provider_cfg());
    let other = list
        .entries
        .iter()
        .find(|e| e.name == "other")
        .expect("other provider present");
    assert!(
        other.model_id.is_empty(),
        "non-active provider without a model field must have an empty model_id, got {:?}",
        other.model_id
    );
    assert!(!other.active);
}

#[test]
fn provider_list_model_id_from_config_for_active_without_model() {
    let list = ProviderList::new(&multi_provider_cfg());
    let active = list
        .entries
        .iter()
        .find(|e| e.name == "active")
        .expect("active provider present");
    assert_eq!(
        active.model_id, "active-model",
        "active provider without a model field must derive model_id from config"
    );
    assert!(active.active);
}

#[test]
fn list_enter_opens_form_when_model_id_unset() {
    // Selecting a provider whose model_id is empty must NOT emit a Save
    // (which would produce a suspicious "name/" model), but instead open the
    // edit form so the user can fill in a real model_id.
    let mut list = ProviderList::new(&multi_provider_cfg());
    list.selected = list
        .entries
        .iter()
        .position(|e| e.name == "other")
        .expect("other provider present");

    let mut slot: Option<ModelMenu> = Some(ModelMenu::List(list));
    let outcome = handle_model_key(&mut slot, enter());
    assert!(
        matches!(outcome, ModelOutcome::Idle),
        "Enter on an unset provider must not Save"
    );
    match &slot {
        Some(ModelMenu::Form(f)) => {
            assert_eq!(f.name, "other", "form should be for the selected provider");
            assert!(
                f.model_id.is_empty(),
                "form model_id should start empty for the user to fill in"
            );
            assert_eq!(
                f.focus,
                ProviderField::ModelId,
                "focus should land on the model_id field to guide the user"
            );
        }
        _ => panic!("expected Form after Enter on unset provider, got a non-Form variant"),
    }
}
/// The armed confirmation renders the selected scope, save target, model name,
/// and save/session-only/cancel hints in a centered dialog.
#[test]
fn save_default_confirm_renders_visible_dialog() {
    let cfg = provider_cfg();
    let mut list = ProviderList::new(&cfg);
    list.confirm_save_default = Some(serde_json::json!({ "model": "deepseek/deepseek-chat" }));
    let menu = ModelMenu::List(list);

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            render_model_popup(f, area, 20, &menu);
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    let mut all = String::new();
    for y in 0..24u16 {
        for x in 0..80u16 {
            if let Some(c) = buf.cell((x, y)) {
                all.push_str(c.symbol());
            }
        }
        all.push('\n');
    }
    assert!(
        all.contains("Save model default?"),
        "confirm title missing:\n{all}"
    );
    assert!(all.contains("Act default?"), "scope missing:\n{all}");
    assert!(
        all.contains("Legacy automatic save target"),
        "save target missing:\n{all}"
    );
    assert!(
        all.contains("session-only"),
        "session-only hint missing:\n{all}"
    );
    assert!(all.contains("deepseek-chat"), "model name missing:\n{all}");
}
