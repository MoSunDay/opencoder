//! `ProviderPatch` serialization: JSON shape, per-provider `model`
//! persistence across a save/load cycle, and the switch/delete helpers.

use crate::model_menu::list::ProviderList;

// ── ProviderPatch ─────────────────────────────────────────────────────────

#[test]
fn provider_patch_wraps_env_var_name_in_braces() {
    let p = crate::model_menu::patch::ProviderPatch {
        protocol: "chat_completions".into(),
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
        protocol: "chat_completions".into(),
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
        protocol: "chat_completions".into(),
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
        protocol: "chat_completions".into(),
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
    let s = crate::model_menu::patch::switch_mode_provider_json(
        opencoder_core::ConfigMode::Shared,
        "foo",
        "bar",
    );
    assert_eq!(
        s["modes"]["act"]["model"],
        serde_json::json!("foo/bar"),
        "Shared normalizes to Act: /model never writes the top-level default"
    );
    assert!(s["model"].is_null());
    let plan = crate::model_menu::patch::switch_mode_provider_json(
        opencoder_core::ConfigMode::Plan,
        "foo",
        "bar",
    );
    assert_eq!(plan["modes"]["plan"]["model"], serde_json::json!("foo/bar"));
    assert!(plan["modes"]["act"].is_null());
}

/// End-to-end regression: a provider created via the menu (ProviderForm →
/// ProviderPatch → to_json → Config::save → Config::load) must retain its
/// model_id in the provider entry. Previously `to_json` omitted the per-
/// provider `model` field, so reopening `/model` lost the model_id and fell
/// back to the active model's id — switching then wrote the wrong model.
#[test]
fn provider_patch_model_survives_save_load_cycle() {
    use opencoder_core::Config;

    let dir = tempfile::tempdir().unwrap();
    // Isolate config discovery to this tempdir on this thread only (no
    // process-env mutation → no `set_var` UB). The thread-local override also
    // disables env overlays (`OPENCODER_MODEL` etc.) so `Config::load`'s
    // `apply_env` can't clobber the saved model with a host env var.
    let _home = super::common::lock_home(dir.path());
    // Simulate adding a provider while a *different* model is active.
    let patch = crate::model_menu::patch::ProviderPatch {
        protocol: "chat_completions".into(),
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
    let switch = serde_json::json!({"model": "other/other-model"});
    Config::save(dir.path(), &switch).unwrap();
    let cfg = Config::load(dir.path()).unwrap();
    assert_eq!(cfg.model_id(), "other-model", "active model changed");

    let list = ProviderList::new(&cfg);
    let entry = list.entries.iter().find(|e| e.name == "qwen3").unwrap();
    assert_eq!(
        entry.model_id, "qwen-max",
        "ProviderList must read the persisted per-provider model, not the active model"
    );
}
