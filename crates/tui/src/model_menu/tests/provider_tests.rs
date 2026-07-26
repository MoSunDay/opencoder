//! Tests for ProviderPatch, ProviderList CRUD, ProviderForm, and headers.

use super::common::{enter, esc, key, provider_cfg};
use crate::model_menu::list::ProviderList;
use crate::model_menu::provider_form::{ProviderField, ProviderForm};
use crate::model_menu::render_model_popup;
use crate::model_menu::state::{handle_model_key, ModelMenu, ModelOutcome};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

// ── ProviderPatch ─────────────────────────────────────────────────────────

#[test]
fn provider_patch_wraps_env_var_name_in_braces() {
    let p = crate::model_menu::patch::ProviderPatch {
        name: "deepseek".into(),
        model_id: "chat".into(),
        base_url: "u".into(),
        api_key: Some("MY_KEY".into()),
        headers: vec![],
    };
    let v = p.to_json();
    assert_eq!(v["model"], serde_json::json!("deepseek/chat"));
    assert_eq!(
        v["providers"]["deepseek"]["api_key"],
        serde_json::json!("{MY_KEY}")
    );
    // model_id must be persisted per-provider so ProviderList can recover it
    // on reopen (regression: was lost, causing wrong model after /model switch).
    assert_eq!(
        v["providers"]["deepseek"]["model"],
        serde_json::json!("chat")
    );
}

#[test]
fn provider_patch_omits_api_key_when_untouched() {
    let p = crate::model_menu::patch::ProviderPatch {
        name: "deepseek".into(),
        model_id: "chat".into(),
        base_url: "u".into(),
        api_key: None,
        headers: vec![],
    };
    let v = p.to_json();
    let has_key = v["providers"]["deepseek"]
        .as_object()
        .unwrap()
        .contains_key("api_key");
    assert!(!has_key, "untouched api_key must be absent from patch");
}

#[test]
fn provider_patch_clears_api_key_when_empty() {
    let p = crate::model_menu::patch::ProviderPatch {
        name: "deepseek".into(),
        model_id: "chat".into(),
        base_url: "u".into(),
        api_key: Some("".into()),
        headers: vec![],
    };
    let v = p.to_json();
    assert_eq!(
        v["providers"]["deepseek"]["api_key"],
        serde_json::Value::Null
    );
}

#[test]
fn provider_patch_serializes_headers() {
    let p = crate::model_menu::patch::ProviderPatch {
        name: "svc".into(),
        model_id: "m".into(),
        base_url: "u".into(),
        api_key: Some("k".into()),
        headers: vec![
            ("X-Tenant".into(), "42".into()),
            ("X-Trace".into(), "abc".into()),
        ],
    };
    let v = p.to_json();
    let hdrs = v["providers"]["svc"]["headers"].as_array().unwrap();
    assert_eq!(hdrs.len(), 2);
    assert_eq!(hdrs[0]["name"], serde_json::json!("X-Tenant"));
    assert_eq!(hdrs[0]["value"], serde_json::json!("42"));
}

#[test]
fn delete_and_switch_patches() {
    let d = crate::model_menu::patch::delete_provider_json("foo");
    assert_eq!(d["providers"]["foo"], serde_json::Value::Null);
    let s = crate::model_menu::patch::switch_provider_json("foo", "bar");
    assert_eq!(s["model"], serde_json::json!("foo/bar"));
}

// ── ProviderList ──────────────────────────────────────────────────────────

/// End-to-end regression: a provider created via the menu (ProviderForm →
/// ProviderPatch → to_json → Config::save → Config::load) must retain its
/// model_id in the provider entry. Previously `to_json` omitted the per-
/// provider `model` field, so reopening `/model` lost the model_id and fell
/// back to the active model's id — switching then wrote the wrong model.
#[test]
fn provider_patch_model_survives_save_load_cycle() {
    use opencoder_core::Config;

    // Scrub env vars that Config::load → apply_env would inject.
    let _model = std::env::var("OPENCODER_MODEL").ok();
    std::env::remove_var("OPENCODER_MODEL");

    let dir = tempfile::tempdir().unwrap();
    // Isolate HOME/XDG_CONFIG_HOME so Config::save_target's global candidates
    // (~~/.opencoder/config.json, XDG) resolve inside this tempdir instead of
    // falling through to the real ~/.opencoder and clobbering the user config.
    let _home = super::common::lock_home(dir.path());
    // Simulate adding a provider while a *different* model is active.
    let patch = crate::model_menu::patch::ProviderPatch {
        name: "qwen3".into(),
        model_id: "qwen-max".into(),
        base_url: "https://api.qwen.com/v1".into(),
        api_key: Some("sk-test".into()),
        headers: vec![],
    };
    let json = patch.to_json();
    assert_eq!(
        json["providers"]["qwen3"]["model"],
        serde_json::json!("qwen-max"),
        "to_json must persist model per-provider"
    );

    Config::save(dir.path(), &json).unwrap();
    let cfg = Config::load(dir.path()).unwrap();
    // Active model is qwen3/qwen-max, not glm — so the fallback in
    // ProviderList would NOT produce qwen-max unless the provider carries it.
    assert_eq!(cfg.model_id(), "qwen-max");

    // Now switch active model to something else and reopen the menu:
    // the qwen3 entry must still show qwen-max, not the active model.
    let switch = crate::model_menu::patch::switch_provider_json("other", "other-model");
    Config::save(dir.path(), &switch).unwrap();
    let cfg = Config::load(dir.path()).unwrap();
    assert_eq!(cfg.model_id(), "other-model", "active model changed");

    let list = ProviderList::new(&cfg);
    let entry = list.entries.iter().find(|e| e.name == "qwen3").unwrap();
    assert_eq!(
        entry.model_id, "qwen-max",
        "ProviderList must read the persisted per-provider model, not the active model"
    );

    // Restore env
    if let Some(v) = _model {
        std::env::set_var("OPENCODER_MODEL", v);
    }
}

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
            assert_eq!(json["model"], serde_json::json!("deepseek/deepseek-chat"));
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
            assert_eq!(json["model"], serde_json::json!("deepseek/deepseek-chat"));
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
            assert_eq!(json["model"], serde_json::json!("deepseek/deepseek-chat"));
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

// ── ProviderForm ──────────────────────────────────────────────────────────

#[test]
fn provider_form_save_produces_patch() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::List(ProviderList::new(&provider_cfg())));
    handle_model_key(&mut slot, key('e')); // -> Form
                                           // Set focus to Save explicitly and press Enter
    {
        let f = match slot.as_mut() {
            Some(ModelMenu::Form(f)) => f,
            _ => unreachable!(),
        };
        f.focus = ProviderField::Save;
    }
    match handle_model_key(&mut slot, enter()) {
        ModelOutcome::Save(json) => {
            assert_eq!(json["model"], serde_json::json!("deepseek/deepseek-chat"));
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
            base_url: "https://active.example.com/v1".to_string(),
            api_key: Some("ak-active".to_string()),
            model: None,
            headers: Vec::new(),
        },
    );
    c.providers.insert(
        "other".to_string(),
        opencoder_core::ProviderConfig {
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

/// When `confirm_save_default` is armed, a centered confirmation dialog with the
/// model name and the y/global + n/session-only + Esc/cancel hints is rendered
/// into the buffer (regression guard: previously only the title bar changed).
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
        all.contains("Save as default"),
        "confirm title missing:\n{all}"
    );
    assert!(all.contains("global"), "global hint missing:\n{all}");
    assert!(
        all.contains("session-only"),
        "session-only hint missing:\n{all}"
    );
    assert!(all.contains("deepseek-chat"), "model name missing:\n{all}");
}
