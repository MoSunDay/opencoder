//! Tests for ConfigPatch serialization and ConfigForm key handling.

use super::common::{backspace, cfg, enter, key, left, right};
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
        context_limit: 128_000,
        fps: 25,
        capabilities_browser: true,
        capabilities_computer_use: false,
        capabilities_tools_subagent: false,
        ap_enabled: true,
        ap_max_iter: 15,
        ap_skill: Some("commit".into()),
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
    assert_eq!(v["capabilities"]["browser"], serde_json::json!(true));
    assert_eq!(v["autopilot"]["enabled"], serde_json::json!(true));
    assert_eq!(v["autopilot"]["max_iterations"], serde_json::json!(15));
    assert_eq!(v["autopilot"]["skill"], serde_json::json!("commit"));
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
        capabilities_browser: false,
        capabilities_computer_use: false,
        capabilities_tools_subagent: false,
        ap_enabled: false,
        ap_max_iter: 10,
        ap_skill: None,
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
        ConfigField::ContextSize,
        ConfigField::Threshold,
        ConfigField::Fps,
        ConfigField::Browser,
        ConfigField::ComputerUse,
        ConfigField::ToolsSubagent,
        ConfigField::ApEnabled,
        ConfigField::ApMaxIter,
        ConfigField::ApSkill,
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
    f.fps = 2;
    f.paste_into("4");
    assert_eq!(f.fps, 24, "2 -> append 4 -> 24");
    f.fps = 2;
    f.paste_into("99");
    assert_eq!(f.fps, 30, "clamped to 30");
}

#[test]
fn config_form_paste_into_threshold() {
    let mut f = ConfigForm::new(&cfg());
    f.focus = ConfigField::Threshold;
    f.threshold = 1000;
    f.paste_into("000");
    assert_eq!(f.threshold, 1_000_000, "1000 -> append 000 -> 1000000");
}

#[test]
fn config_form_inits_autopilot_from_config() {
    let mut c = cfg();
    c.autopilot.enabled = true;
    c.autopilot.max_iterations = 7;
    c.autopilot.skill = Some("reviewer".into());
    let f = ConfigForm::new(&c);
    assert!(f.ap_enabled);
    assert_eq!(f.ap_max_iter, 7);
    assert_eq!(f.ap_skill_input, "reviewer");
    let p = f.build_patch();
    assert!(p.ap_enabled);
    assert_eq!(p.ap_max_iter, 7);
    assert_eq!(p.ap_skill.as_deref(), Some("reviewer"));
}

#[test]
fn config_form_empty_skill_produces_none() {
    let f = ConfigForm::new(&cfg());
    assert!(f.ap_skill_input.is_empty());
    assert!(f.build_patch().ap_skill.is_none());
}

#[test]
fn config_form_toggle_ap_enabled() {
    let mut slot: Option<ModelMenu> = Some(ModelMenu::Config(ConfigForm::new(&cfg())));
    {
        let f = match slot.as_mut().unwrap() {
            ModelMenu::Config(f) => f,
            _ => unreachable!(),
        };
        f.focus = ConfigField::ApEnabled;
    }
    let before = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f.ap_enabled,
        _ => unreachable!(),
    };
    handle_model_key(&mut slot, right());
    let after = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f.ap_enabled,
        _ => unreachable!(),
    };
    assert_eq!(after, !before, "Right toggles ap_enabled");
}

#[test]
fn config_form_inits_context_size_from_config() {
    let f = ConfigForm::new(&cfg());
    assert_eq!(f.context_size, 128_000, "default context size is 128k");

    let mut c = cfg();
    c.context_limit = Some(200_000);
    let f = ConfigForm::new(&c);
    assert_eq!(f.context_size, 200_000);
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
        f.context_size = 0;
    }
    handle_model_key(&mut slot, key('2'));
    let f = match slot.as_ref().unwrap() {
        ModelMenu::Config(f) => f,
        _ => unreachable!(),
    };
    assert_eq!(f.context_size, 2);
}

#[test]
fn backspace_pops_digit_from_threshold() {
    let mut f = ConfigForm::new(&cfg());
    f.focus = ConfigField::Threshold;
    f.threshold = 50_000;
    let (_outcome, next) = crate::model_menu::config_form::handle_key(f, backspace());
    let f = match next {
        Some(ModelMenu::Config(f)) => f,
        _ => panic!("expected Config menu"),
    };
    assert_eq!(f.threshold, 5_000, "50_000 / 10 = 5_000");
}

#[test]
fn backspace_pops_digit_from_context_size() {
    let mut f = ConfigForm::new(&cfg());
    f.focus = ConfigField::ContextSize;
    f.context_size = 50_000;
    let (_outcome, next) = crate::model_menu::config_form::handle_key(f, backspace());
    let f = match next {
        Some(ModelMenu::Config(f)) => f,
        _ => panic!("expected Config menu"),
    };
    assert_eq!(f.context_size, 5_000, "50_000 / 10 = 5_000");
}

#[test]
fn validate_rejects_threshold_above_context_size() {
    let mut f = ConfigForm::new(&cfg());
    f.threshold = 200_000;
    f.context_size = 128_000;
    // validate is private, so trigger it via handle_key on Save.
    f.focus = ConfigField::Save;
    let (outcome, next) = crate::model_menu::config_form::handle_key(f, enter());
    // Should NOT save; should stay as Config with an error.
    assert!(matches!(outcome, ModelOutcome::Idle), "save should be blocked");
    assert!(next.is_some(), "menu should stay open on validation error");
}

#[test]
fn config_patch_writes_context_limit() {
    let mut f = ConfigForm::new(&cfg());
    f.context_size = 96_000;
    let v = f.build_patch().to_json();
    assert_eq!(v["context_limit"], serde_json::json!(96_000));
}
