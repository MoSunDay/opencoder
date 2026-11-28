//! `ProviderForm` behaviour: save patch, API-key masking, Esc cancel, paste
//! routing, and Ctrl+L / Ctrl+U field clearing.

use super::common::{ctrl, enter, esc, key, provider_cfg};
use crate::model_menu::list::ProviderList;
use crate::model_menu::provider_form::{ProviderField, ProviderForm};
use crate::model_menu::state::{handle_model_key, ModelMenu, ModelOutcome};

// ── ProviderForm ──────────────────────────────────────────────────────────

#[test]
fn provider_form_save_produces_patch() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::List(ProviderList::new(&provider_cfg())));
    handle_model_key(&mut slot, key('e')); // -> Form
    {
        let f = match slot.as_mut() {
            Some(ModelMenu::Form(f)) => f,
            _ => unreachable!(),
        };
        f.focus = ProviderField::Save;
    }
    assert!(matches!(
        handle_model_key(&mut slot, enter()),
        ModelOutcome::Idle
    ));
    match handle_model_key(&mut slot, key('y')) {
        ModelOutcome::Save(json) => {
            assert!(
                json["model"].is_null() && json.get("modes").is_none(),
                "a scoped provider edit never selects a model (Shared or Act/Plan): {json}"
            );
            assert_eq!(
                json["providers"]["deepseek"]["base_url"],
                serde_json::json!("https://api.deepseek.com/v1")
            );
            let hdrs = json["providers"]["deepseek"]["headers"].as_array().unwrap();
            assert_eq!(hdrs.len(), 1);
        }
        _ => panic!("Save should produce Save outcome"),
    }
}

#[test]
fn provider_form_api_key_editing() {
    let mut form = ProviderForm::from_existing("svc", "u", "m", "orig-key-12345", vec![]);
    form.focus = ProviderField::ApiKey;
    assert_eq!(form.api_key_display(), "or****2345");
    form.api_key_input.push('n');
    form.api_key_edited = true;
    assert_eq!(form.api_key_display(), "*");
    assert_eq!(form.resolve_api_key(), Some("n".into()));
}

// ── Cancel ────────────────────────────────────────────────────────────────

#[test]
fn esc_cancels_any_mode() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::Config(
        crate::model_menu::config_form::ConfigForm::new(&super::common::cfg()),
    ));
    assert!(matches!(
        handle_model_key(&mut slot, esc()),
        ModelOutcome::Cancel
    ));
    assert!(slot.is_none());

    let mut slot: Option<ModelMenu> = Some(ModelMenu::List(ProviderList::new(&provider_cfg())));
    assert!(matches!(
        handle_model_key(&mut slot, esc()),
        ModelOutcome::Cancel
    ));
    assert!(slot.is_none());
}

// ── paste routing (ProviderForm / ModelMenu::Form) ────────────────────────

#[test]
fn provider_form_paste_into_api_key() {
    let mut form = ProviderForm::from_existing("svc", "u", "m", "orig-key-12345", vec![]);
    form.focus = ProviderField::ApiKey;
    form.paste_into("sk-pasted-secret");
    assert_eq!(form.api_key_input, "sk-pasted-secret");
    assert!(
        form.api_key_edited,
        "paste should mark the api key as edited"
    );
    assert_eq!(form.resolve_api_key(), Some("sk-pasted-secret".into()));
}

#[test]
fn provider_form_paste_appends_to_model_id() {
    let mut form = ProviderForm::from_existing("svc", "u", "m", "orig", vec![]);
    form.focus = ProviderField::ModelId;
    form.paste_into("-preview");
    assert_eq!(form.model_id, "m-preview");
}

#[test]
fn provider_form_paste_skips_readonly_name() {
    let mut form = ProviderForm::from_existing("svc", "u", "m", "orig", vec![]);
    // from_existing marks the name read-only.
    form.focus = ProviderField::Name;
    form.paste_into("ignored");
    assert_eq!(form.name, "svc", "a read-only name must not accept paste");
}

#[test]
fn provider_form_paste_into_base_url() {
    let mut form = ProviderForm::new_blank(&provider_cfg());
    form.focus = ProviderField::BaseUrl;
    let before = form.base_url.clone();
    form.paste_into("/v2");
    assert_eq!(form.base_url, format!("{}/v2", before));
}

#[test]
fn model_menu_paste_routes_to_provider_form_field() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::Form(ProviderForm::from_existing(
        "svc",
        "u",
        "m",
        "orig",
        vec![],
    )));
    {
        let f = match slot.as_mut() {
            Some(ModelMenu::Form(f)) => f,
            _ => unreachable!(),
        };
        f.focus = ProviderField::BaseUrl;
    }
    if let Some(menu) = slot.as_mut() {
        menu.paste("https://example.com/v1");
    }
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Form(f) => f,
        _ => unreachable!(),
    };
    assert_eq!(f.base_url, "uhttps://example.com/v1");
}

#[test]
fn model_menu_paste_is_a_noop_in_list() {
    // The List variant has no text fields; paste must not panic and must leave
    // the menu unchanged.
    let mut slot: Option<ModelMenu> = Some(ModelMenu::List(ProviderList::new(&provider_cfg())));
    if let Some(menu) = slot.as_mut() {
        menu.paste("anything");
    }
    assert!(matches!(slot, Some(ModelMenu::List(_))));
}

// ── Ctrl+L / Ctrl+U clear focused field ───────────────────────────────────

#[test]
fn ctrl_clear_empties_model_id_and_base_url() {
    let mut slot: Option<ModelMenu> =
        Some(ModelMenu::Form(ProviderForm::new_blank(&provider_cfg())));
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Form(f) => f,
            _ => unreachable!(),
        };
        f.focus = ProviderField::ModelId;
        f.model_id = "glm-5.2".into();
    }
    handle_model_key(&mut slot, ctrl('u'));
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Form(f) => f,
            _ => unreachable!(),
        };
        assert!(f.model_id.is_empty(), "Ctrl+U must clear model_id");
        f.focus = ProviderField::BaseUrl;
        f.base_url = "https://open.bigmodel.cn/v4".into();
    }
    handle_model_key(&mut slot, ctrl('l'));
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Form(f) => f,
        _ => unreachable!(),
    };
    assert!(f.base_url.is_empty(), "Ctrl+L must clear base_url");
    assert_eq!(f.focus, ProviderField::BaseUrl, "focus must stay");
}

#[test]
fn ctrl_clear_editable_name_but_readonly_name_is_noop() {
    // New provider: editable name clears.
    let mut slot: Option<ModelMenu> =
        Some(ModelMenu::Form(ProviderForm::new_blank(&provider_cfg())));
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Form(f) => f,
            _ => unreachable!(),
        };
        f.name = "acme".into();
    }
    handle_model_key(&mut slot, ctrl('u'));
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Form(f) => f,
        _ => unreachable!(),
    };
    assert!(f.name.is_empty(), "Ctrl+U must clear editable name");

    // Editing an existing provider: name is read-only → no-op.
    let mut slot: Option<ModelMenu> = Some(ModelMenu::Form(ProviderForm::from_existing(
        "acme",
        "u",
        "m",
        "k",
        vec![],
    )));
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Form(f) => f,
            _ => unreachable!(),
        };
        f.focus = ProviderField::Name;
    }
    handle_model_key(&mut slot, ctrl('l'));
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Form(f) => f,
        _ => unreachable!(),
    };
    assert_eq!(f.name, "acme", "read-only name must survive Ctrl+L");
}

#[test]
fn ctrl_clear_api_key_marks_edited() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::Form(ProviderForm::from_existing(
        "svc",
        "u",
        "m",
        "orig-key-12345",
        vec![],
    )));
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Form(f) => f,
            _ => unreachable!(),
        };
        f.focus = ProviderField::ApiKey;
    }
    assert!(!slot_edited(&slot));
    handle_model_key(&mut slot, ctrl('u'));
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Form(f) => f,
        _ => unreachable!(),
    };
    assert!(
        f.api_key_edited,
        "Ctrl+U on api_key must mark it edited (persist the clear)"
    );
    assert!(
        f.api_key_input.is_empty(),
        "api_key raw buffer must be empty after clear"
    );
}

fn slot_edited(slot: &Option<ModelMenu>) -> bool {
    match slot {
        Some(ModelMenu::Form(f)) => f.api_key_edited,
        _ => false,
    }
}

#[test]
fn ctrl_d_still_quits_in_provider_form() {
    let mut slot: Option<ModelMenu> =
        Some(ModelMenu::Form(ProviderForm::new_blank(&provider_cfg())));
    let out = handle_model_key(&mut slot, ctrl('d'));
    assert!(matches!(out, ModelOutcome::Quit), "Ctrl+D must quit modal");
    assert!(slot.is_none());
}
