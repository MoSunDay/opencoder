//! Tests for ConfigPatch serialization and ConfigForm key handling.

use super::common::{cfg, enter, key, left, right};
use crate::model_menu::config_form::{ConfigField, ConfigForm};
use crate::model_menu::patch::ConfigPatch;
use crate::model_menu::state::{handle_model_key, ModelMenu, ModelOutcome};

// ── ConfigPatch ───────────────────────────────────────────────────────────

#[test]
fn config_patch_serializes_all_fields() {
    let p = ConfigPatch {
        reasoning_effort: Some("high".into()),
        interleaved_thinking: Some(true),
        max_tokens: Some(8192),
        context_threshold: 80_000,
        fps: 25,
        capabilities_browser: true,
        capabilities_computer_use: false,
        capabilities_tools_subagent: false,
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
    assert_eq!(v["capabilities"]["browser"], serde_json::json!(true));
}

#[test]
fn config_patch_omits_max_tokens_when_none() {
    let p = ConfigPatch {
        reasoning_effort: None,
        interleaved_thinking: None,
        max_tokens: None,
        context_threshold: 1000,
        fps: 10,
        capabilities_browser: false,
        capabilities_computer_use: false,
        capabilities_tools_subagent: false,
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
    assert_eq!(f.fps, 10);
    assert_eq!(f.build_patch().fps, 10);
}

#[test]
fn config_form_inits_capabilities_from_config() {
    let mut c = cfg();
    c.capabilities.browser = true;
    c.capabilities.computer_use = true;
    let f = ConfigForm::new(&c);
    assert!(f.capabilities_browser);
    assert!(f.capabilities_computer_use);
    let p = f.build_patch();
    assert!(p.capabilities_browser);
    assert!(p.capabilities_computer_use);
}

#[test]
fn enter_chains_through_config_fields_to_save() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::Config(ConfigForm::new(&cfg())));
    let order = [
        ConfigField::InterleavedThinking,
        ConfigField::MaxTokens,
        ConfigField::Threshold,
        ConfigField::Fps,
        ConfigField::Browser,
        ConfigField::ComputerUse,
        ConfigField::ToolsSubagent,
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
fn typing_digits_sets_fps() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::Config(ConfigForm::new(&cfg())));
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Config(f) => f,
            _ => unreachable!(),
        };
        f.focus = ConfigField::Fps;
        f.fps = 2;
    }
    handle_model_key(&mut slot, key('4'));
    let fps = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f.fps,
        _ => unreachable!(),
    };
    assert_eq!(fps, 24, "from fps=2, typing '4' yields 24");
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
