use opencoder_core::{scoped_config_home, Config, ConfigMode};
use serde_json::{json, Value};

fn configured(value: Value) -> Config {
    Config::default().merged_with(&value)
}

#[test]
fn missing_modes_preserves_the_exact_legacy_config_view() {
    let config = configured(json!({
        "model": "legacy/provider-model",
        "reasoning_effort": "medium",
        "interleaved_thinking": false,
        "max_tokens": 1234,
        "context_limit": 5678,
        "cache_salt": false
    }));
    let legacy = serde_json::to_value(&config).unwrap();
    assert!(legacy.get("modes").is_none(), "empty modes stay absent");

    for mode in [ConfigMode::Shared, ConfigMode::Act, ConfigMode::Plan] {
        let effective = config.effective_for_mode(mode);
        assert_eq!(serde_json::to_value(effective).unwrap(), legacy);
    }
}

#[test]
fn shared_is_the_base_and_act_plan_overrides_are_independent() {
    let config = configured(json!({
        "model": "legacy/base-model",
        "reasoning_effort": "low",
        "max_tokens": 100,
        "context_limit": 10_000,
        "interleaved_thinking": true,
        "cache_salt": true,
        "modes": {
            "shared": {
                "model": "shared/shared-model",
                "reasoning_effort": "medium",
                "max_tokens": 200,
                "context_limit": 20_000,
                "interleaved_thinking": false,
                "cache_salt": false
            },
            "act": { "max_tokens": 300 },
            "plan": {
                "model": "plan/plan-model",
                "reasoning_effort": "high",
                "context_limit": 30_000
            }
        }
    }));

    let shared = config.effective_for_mode(ConfigMode::Shared);
    assert_eq!(shared.model, "shared/shared-model");
    assert_eq!(shared.provider_id(), "shared");
    assert_eq!(shared.model_id(), "shared-model");
    assert_eq!(shared.max_tokens, Some(200));
    assert_eq!(shared.context_limit(), 20_000);

    let act = config.effective_for_mode(ConfigMode::Act);
    assert_eq!(act.model, "shared/shared-model");
    assert_eq!(act.reasoning_effort.as_deref(), Some("medium"));
    assert_eq!(act.max_tokens, Some(300));
    assert_eq!(act.context_limit(), 20_000);
    assert_eq!(act.interleaved_thinking, Some(false));
    assert_eq!(act.cache_salt, Some(false));

    let plan = config.effective_for_mode(ConfigMode::Plan);
    assert_eq!(plan.model, "plan/plan-model");
    assert_eq!(plan.provider_id(), "plan");
    assert_eq!(plan.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(plan.max_tokens, Some(200), "Plan inherits Shared, not Act");
    assert_eq!(plan.context_limit(), 30_000);
    assert_eq!(plan.interleaved_thinking, Some(false));
    assert_eq!(config.model, "legacy/base-model", "resolution is pure");
}

#[test]
fn explicit_model_overrides_every_mode_without_clearing_generation_fields() {
    let mut config = configured(json!({
        "model": "cli/override-model",
        "providers": {
            "cli": { "base_url": "https://example.invalid/v1", "api_key": "placeholder" }
        },
        "modes": {
            "shared": { "model": "shared/model", "max_tokens": 200 },
            "act": { "model": "act/model", "reasoning_effort": "low" },
            "plan": { "model": "plan/model", "context_limit": 30_000 }
        }
    }));
    let provider = config.providers["cli"].clone();

    assert!(
        config.apply_explicit_model("cli/override-model"),
        "mode models must change even when the legacy model already matches"
    );
    for mode in [ConfigMode::Shared, ConfigMode::Act, ConfigMode::Plan] {
        assert_eq!(config.effective_for_mode(mode).model, "cli/override-model");
    }
    assert_eq!(config.modes.shared.max_tokens, Some(200));
    assert_eq!(
        config
            .modes
            .act
            .as_ref()
            .unwrap()
            .reasoning_effort
            .as_deref(),
        Some("low")
    );
    assert_eq!(
        config.modes.plan.as_ref().unwrap().context_limit,
        Some(30_000)
    );
    assert_eq!(config.providers["cli"], provider);
    assert!(config.modes.shared.model.is_none());
    assert!(config.modes.act.as_ref().unwrap().model.is_none());
    assert!(config.modes.plan.as_ref().unwrap().model.is_none());
    assert!(!config.apply_explicit_model("cli/override-model"));
}

#[test]
fn null_deletion_restores_shared_inheritance_in_memory() {
    let config = configured(json!({
        "model": "legacy/base-model",
        "modes": {
            "shared": { "model": "shared/shared-model", "max_tokens": 200 },
            "act": { "model": "act/act-model", "max_tokens": 400 }
        }
    }));
    let merged = config.merged_with(&json!({
        "modes": { "act": { "model": null, "max_tokens": null } }
    }));
    let stored = serde_json::to_value(&merged.modes).unwrap();
    assert!(stored["act"].get("model").is_none());
    assert!(stored["act"].get("max_tokens").is_none());

    let act = merged.effective_for_mode(ConfigMode::Act);
    assert_eq!(act.model, "shared/shared-model");
    assert_eq!(act.max_tokens, Some(200));

    let cleared = merged.merged_with(&json!({"modes": null}));
    assert!(cleared.modes.is_empty());
    assert_eq!(
        cleared.effective_for_mode(ConfigMode::Act).model,
        "legacy/base-model"
    );
}

#[test]
fn disk_patches_deep_merge_and_delete_nested_mode_overrides() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _guard = scoped_config_home(home.path().to_path_buf());
    let target = work.path().join("opencoder.json");

    Config::save(
        work.path(),
        &json!({
            "model": "legacy/base-model",
            "modes": {
                "shared": { "model": "shared/shared-model", "max_tokens": 200 },
                "act": { "model": "act/act-model", "reasoning_effort": "low" }
            }
        }),
    )
    .unwrap();
    Config::save(
        work.path(),
        &json!({"modes": {"act": {"reasoning_effort": "high"}}}),
    )
    .unwrap();
    let merged: Value = serde_json::from_str(&std::fs::read_to_string(&target).unwrap()).unwrap();
    assert_eq!(merged["modes"]["act"]["model"], "act/act-model");
    assert_eq!(merged["modes"]["act"]["reasoning_effort"], "high");
    assert_eq!(merged["modes"]["shared"]["max_tokens"], 200);

    Config::save(work.path(), &json!({"modes": {"act": {"model": null}}})).unwrap();
    let deleted: Value = serde_json::from_str(&std::fs::read_to_string(&target).unwrap()).unwrap();
    assert!(deleted["modes"]["act"].get("model").is_none());
    assert_eq!(deleted["modes"]["act"]["reasoning_effort"], "high");
    let loaded = Config::load(work.path()).unwrap();
    assert_eq!(
        loaded.effective_for_mode(ConfigMode::Act).model,
        "shared/shared-model"
    );
}

#[test]
fn malformed_nested_model_patches_are_rejected_before_writing() {
    for mode in ["shared", "act", "plan"] {
        let home = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let _guard = scoped_config_home(home.path().to_path_buf());
        let patch = json!({"modes": {mode: {"model": "m/g"}}});
        let error = Config::save(work.path(), &patch).unwrap_err();
        assert!(
            error.to_string().contains(&format!("modes.{mode}.model")),
            "unexpected error: {error}"
        );
        assert!(!work.path().join("opencoder.json").exists());

        let wrong_type = json!({"modes": {mode: {"model": 42}}});
        assert!(Config::save(work.path(), &wrong_type).is_err());
        assert!(!work.path().join("opencoder.json").exists());
    }
}
