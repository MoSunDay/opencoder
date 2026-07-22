//! P1 functional tests for Config: deep-merge precedence, env resolution,
//! small_model / context_limit plumbing, and the {VAR} secret indirection.

use std::fs;
use std::sync::Mutex;

use opencoder_core::Config;

// Env mutation is process-global; serialize tests that touch the environment.
static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn merge_project_file_overrides_defaults() {
    let _g = ENV_LOCK.lock().unwrap();
    let (_home_guard, dir) = isolated_home();
    fs::write(
        dir.path().join("opencoder.json"),
        r#"{
            "model": "zhipuai-coding-plan/glm-5.2",
            "small_model": "cheap/mini",
            "context_limit": 60000,
            "compaction": { "auto": true, "context_threshold": 40000, "reserved": 8000, "tail_turns": 3 }
        }"#,
    )
    .unwrap();

    let cfg = Config::load(dir.path()).unwrap();
    assert_eq!(cfg.model, "zhipuai-coding-plan/glm-5.2");
    assert_eq!(cfg.model_id(), "glm-5.2");
    assert_eq!(cfg.provider_id(), "zhipuai-coding-plan");
    assert_eq!(cfg.small_model.as_deref(), Some("cheap/mini"));
    assert_eq!(cfg.small_model_id(), "mini");
    assert_eq!(cfg.small_model_or_primary(), "mini");
    assert_eq!(cfg.context_limit(), 60000);
    assert_eq!(cfg.compaction.context_threshold, 40000);
    assert_eq!(cfg.compaction.reserved, 8000);
    assert_eq!(cfg.compaction.tail_turns, 3);
}

#[test]
fn env_overrides_project_file() {
    let _g = ENV_LOCK.lock().unwrap();
    std::env::set_var("OPENCODER_MODEL", "env/model-from-env");
    std::env::set_var("OPENCODER_SMALL_MODEL", "env/small");
    std::env::set_var("OPENCODER_CONTEXT_LIMIT", "99999");
    std::env::remove_var("OPENAI_BASE_URL");

    let (_home_guard, dir) = isolated_home();
    fs::write(
        dir.path().join("opencoder.json"),
        r#"{"model":"file/model","small_model":"file/sm","context_limit":1000}"#,
    )
    .unwrap();
    let cfg = Config::load(dir.path()).unwrap();

    std::env::remove_var("OPENCODER_MODEL");
    std::env::remove_var("OPENCODER_SMALL_MODEL");
    std::env::remove_var("OPENCODER_CONTEXT_LIMIT");

    assert_eq!(
        cfg.model, "env/model-from-env",
        "OPENCODER_MODEL wins over file"
    );
    assert_eq!(cfg.small_model.as_deref(), Some("env/small"));
    assert_eq!(cfg.context_limit(), 99999);
}

#[test]
fn cache_salt_env_override() {
    // The per-agent prefix-cache salt defaults to Some(true) and is toggled
    // by OPENCODER_CACHE_SALT. This is the one env knob that controls whether
    // the outbound request body carries the `cache_salt` field at all, so its
    // three states (unset/default, false, true) plus override-of-file are
    // asserted here. Body emission itself is covered by request_body.rs and
    // cache_salt.rs; this test pins the config->field link they compose with.
    let _g = ENV_LOCK.lock().unwrap();
    let (_home_guard, dir) = isolated_home();
    fs::write(dir.path().join("opencoder.json"), r#"{"cache_salt":true}"#).unwrap();

    // Unset -> serde default kicks in (Some(true)).
    std::env::remove_var("OPENCODER_CACHE_SALT");
    assert_eq!(
        Config::load(dir.path()).unwrap().cache_salt,
        Some(true),
        "unset -> default Some(true)"
    );

    // =false -> disabled, overriding the file's true.
    std::env::set_var("OPENCODER_CACHE_SALT", "false");
    assert_eq!(
        Config::load(dir.path()).unwrap().cache_salt,
        Some(false),
        "OPENCODER_CACHE_SALT=false wins over file"
    );

    // Truthy aliases also enable.
    for v in ["1", "yes", "true"] {
        std::env::set_var("OPENCODER_CACHE_SALT", v);
        assert_eq!(
            Config::load(dir.path()).unwrap().cache_salt,
            Some(true),
            "OPENCODER_CACHE_SALT={v} -> Some(true)"
        );
    }
    for v in ["0", "no", "false"] {
        std::env::set_var("OPENCODER_CACHE_SALT", v);
        assert_eq!(
            Config::load(dir.path()).unwrap().cache_salt,
            Some(false),
            "OPENCODER_CACHE_SALT={v} -> Some(false)"
        );
    }

    // Garbage value -> ignored, file's true survives.
    std::env::set_var("OPENCODER_CACHE_SALT", "maybe");
    assert_eq!(
        Config::load(dir.path()).unwrap().cache_salt,
        Some(true),
        "unrecognized value is ignored"
    );

    std::env::remove_var("OPENCODER_CACHE_SALT");
}

#[test]
fn braces_api_key_resolves_env_var() {
    let _g = ENV_LOCK.lock().unwrap();
    std::env::set_var("ZHIPU_API_KEY", "secret-value-123");
    let (_home_guard, dir) = isolated_home();
    fs::write(
        dir.path().join("opencoder.json"),
        r#"{"provider":{"api_key":"{ZHIPU_API_KEY}"}}"#,
    )
    .unwrap();
    let cfg = Config::load(dir.path()).unwrap();
    std::env::remove_var("ZHIPU_API_KEY");
    assert_eq!(
        cfg.api_key().unwrap(),
        "secret-value-123",
        "braces var must resolve"
    );
}

#[test]
fn defaults_when_no_config_present() {
    let _g = ENV_LOCK.lock().unwrap();
    let (_home_guard, dir) = isolated_home();
    let cfg = Config::load(dir.path()).unwrap();
    assert_eq!(cfg.context_limit(), opencoder_core::DEFAULT_CONTEXT_LIMIT);
    assert_eq!(cfg.compaction.reserved, 20_000);
    assert!(cfg.compaction.auto);
    assert_eq!(cfg.agent.default, "act");
}

#[test]
fn reserved_saturates_against_context_limit() {
    let _g = ENV_LOCK.lock().unwrap();
    let (_home_guard, dir) = isolated_home();
    fs::write(
        dir.path().join("opencoder.json"),
        r#"{"context_limit": 1000, "compaction": {"reserved": 5000}}"#,
    )
    .unwrap();
    let cfg = Config::load(dir.path()).unwrap();
    // reserved > context_limit must not underflow usable budget in compaction logic;
    // we expose the primitives so the contract is testable.
    let reserved = cfg
        .compaction
        .reserved
        .min(cfg.context_limit().saturating_sub(1));
    let usable = cfg.context_limit().saturating_sub(reserved);
    assert!(
        usable >= 1,
        "usable must stay positive even with over-large reserved"
    );
}

#[test]
fn home_opencoder_config_is_discovered() {
    let _g = ENV_LOCK.lock().unwrap();
    // Point HOME at a temp dir so we don't depend on/clobber the real home.
    let fake_home = tempfile::tempdir().unwrap();
    let prev_home = std::env::var_os("HOME");
    std::env::set_var("HOME", fake_home.path());

    // No project config — load from a clean cwd.
    let cwd = tempfile::tempdir().unwrap();
    let cfg_no_home = Config::load(cwd.path()).unwrap();
    assert_eq!(
        cfg_no_home.model, "openai/gpt-4o-mini",
        "default when no ~/.opencoder"
    );

    // Drop ~/.opencoder/config.json → must be picked up from any cwd.
    let cfg_dir = fake_home.path().join(".opencoder");
    std::fs::create_dir_all(&cfg_dir).unwrap();
    std::fs::write(
        cfg_dir.join("config.json"),
        r#"{"model":"zhipuai-coding-plan/glm-5.2","provider":{"base_url":"https://x.example/v4","api_key":"k"},"max_tokens":4096}"#,
    )
    .unwrap();

    let cfg = Config::load(cwd.path()).unwrap();
    // restore HOME before any assert failure
    match prev_home {
        Some(h) => std::env::set_var("HOME", h),
        None => std::env::remove_var("HOME"),
    }

    assert_eq!(
        cfg.model, "zhipuai-coding-plan/glm-5.2",
        "~/.opencoder/config.json must be discovered"
    );
    assert_eq!(cfg.provider.base_url, "https://x.example/v4");
    assert_eq!(cfg.max_tokens, Some(4096));
}

#[test]
fn reasoning_effort_is_parsed_and_default_none() {
    let _g = ENV_LOCK.lock().unwrap();
    let (_home_guard, dir) = isolated_home();
    fs::write(
        dir.path().join("opencoder.json"),
        r#"{"reasoning_effort":"high"}"#,
    )
    .unwrap();
    let cfg = Config::load(dir.path()).unwrap();
    assert_eq!(cfg.reasoning_effort.as_deref(), Some("high"));
}

#[test]
fn reasoning_effort_defaults_to_none() {
    let _g = ENV_LOCK.lock().unwrap();
    let (_home_guard, dir) = isolated_home();
    let cfg = Config::load(dir.path()).unwrap();
    assert!(
        cfg.reasoning_effort.is_none(),
        "absent reasoning_effort must stay None"
    );
}

#[test]
fn interleaved_thinking_defaults_to_true() {
    let _g = ENV_LOCK.lock().unwrap();
    let (_home_guard, dir) = isolated_home();
    let cfg = Config::load(dir.path()).unwrap();
    assert_eq!(
        cfg.interleaved_thinking,
        Some(true),
        "absent interleaved_thinking must default to Some(true)"
    );
}

#[test]
fn interleaved_thinking_parsed_from_config() {
    let _g = ENV_LOCK.lock().unwrap();
    let (_home_guard, dir) = isolated_home();
    fs::write(
        dir.path().join("opencoder.json"),
        r#"{"interleaved_thinking": false}"#,
    )
    .unwrap();
    let cfg = Config::load(dir.path()).unwrap();
    assert_eq!(cfg.interleaved_thinking, Some(false));
}

#[test]
fn interleaved_thinking_roundtrips_through_save() {
    let _g = ENV_LOCK.lock().unwrap();
    let (_home_guard, dir) = isolated_home();
    let patch = serde_json::json!({ "interleaved_thinking": false });
    Config::save(dir.path(), &patch).unwrap();
    let cfg = Config::load(dir.path()).unwrap();
    assert_eq!(
        cfg.interleaved_thinking,
        Some(false),
        "save → load must preserve interleaved_thinking=false"
    );
}

#[test]
fn save_persists_patch_and_roundtrips() {
    let _g = ENV_LOCK.lock().unwrap();
    let (_home_guard, dir) = isolated_home();
    let patch = serde_json::json!({
        "model": "zhipuai-coding-plan/glm-5.2",
        "provider": { "base_url": "https://open.bigmodel.cn/api/coding/paas/v4", "api_key": "sk-plaintext" },
        "reasoning_effort": "high",
        "compaction": { "context_threshold": 100000 }
    });
    let written = Config::save(dir.path(), &patch).unwrap();
    assert!(
        written.ends_with("opencoder.json"),
        "must save to project-local opencoder.json"
    );

    let cfg = Config::load(dir.path()).unwrap();
    assert_eq!(cfg.model, "zhipuai-coding-plan/glm-5.2");
    assert_eq!(
        cfg.provider.base_url,
        "https://open.bigmodel.cn/api/coding/paas/v4"
    );
    assert_eq!(cfg.provider.api_key.as_deref(), Some("sk-plaintext"));
    assert_eq!(cfg.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(cfg.compaction.context_threshold, 100_000);
}

#[test]
fn save_preserves_unrelated_keys_on_merge() {
    let _g = ENV_LOCK.lock().unwrap();
    let (_home_guard, dir) = isolated_home();
    // Pre-existing file with a sibling compaction key we must NOT clobber.
    fs::write(
        dir.path().join("opencoder.json"),
        r#"{"compaction":{"tail_turns":5,"context_threshold":5000}}"#,
    )
    .unwrap();
    let patch = serde_json::json!({
        "compaction": { "context_threshold": 9000 }
    });
    Config::save(dir.path(), &patch).unwrap();
    let cfg = Config::load(dir.path()).unwrap();
    assert_eq!(
        cfg.compaction.context_threshold, 9000,
        "patched key updates"
    );
    assert_eq!(
        cfg.compaction.tail_turns, 5,
        "sibling key preserved by deep merge"
    );
}

#[test]
fn save_wraps_env_var_name_in_braces() {
    use opencoder_core::looks_like_env_var;
    assert!(looks_like_env_var("ZHIPU_API_KEY"));
    assert!(!looks_like_env_var("sk-abcd"));
    assert!(!looks_like_env_var(""));
    assert!(looks_like_env_var("KEY_1"));
}

#[test]
fn save_refuses_malformed_model_value() {
    // `m/g` would resolve model_id() to "g" and silently break every request;
    // save must refuse to persist it so the bad value does not stick in the
    // config file (mirrors the load-side warn_if_suspicious_model predicate,
    // but as a hard write-path guard).
    let _g = ENV_LOCK.lock().unwrap();
    let (_home_guard, dir) = isolated_home();

    let res = Config::save(dir.path(), &serde_json::json!({ "model": "m/g" }));
    assert!(
        res.is_err(),
        "save must reject a malformed `model` value, not write it"
    );
    let msg = format!("{}", res.unwrap_err());
    assert!(
        msg.contains("malformed") || msg.contains("model"),
        "error should mention the malformed model; got: {msg}"
    );
    // No file should have been written for the rejected value.
    assert!(
        !dir.path().join("opencoder.json").exists(),
        "rejected model must not produce a config file"
    );

    // A well-formed bare model id (no `/`) is still accepted and round-trips.
    Config::save(dir.path(), &serde_json::json!({ "model": "glm-5.2" })).unwrap();
    let cfg = Config::load(dir.path()).unwrap();
    assert_eq!(cfg.model, "glm-5.2");
}

#[test]
fn save_can_remove_reasoning_effort_via_null() {
    // Setting reasoning_effort to null in the patch must REMOVE the field
    // (merge_json treats null as delete) so it is omitted from request bodies.
    let _g = ENV_LOCK.lock().unwrap();
    let (_home_guard, dir) = isolated_home();
    fs::write(
        dir.path().join("opencoder.json"),
        r#"{"model":"demo/model","reasoning_effort":"high"}"#,
    )
    .unwrap();
    let patch = serde_json::json!({ "reasoning_effort": serde_json::Value::Null });
    Config::save(dir.path(), &patch).unwrap();
    let cfg = Config::load(dir.path()).unwrap();
    assert!(
        cfg.reasoning_effort.is_none(),
        "null patch must delete reasoning_effort"
    );
}

#[test]
fn save_env_var_api_key_roundtrips_through_resolve() {
    // {ENV} api_key written by the menu must survive save → load → resolve_env.
    let _g = ENV_LOCK.lock().unwrap();
    let (_home_guard, dir) = isolated_home();
    std::env::set_var("MY_TEST_KEY", "resolved-secret");
    let patch = serde_json::json!({
        "model": "demo/model",
        "provider": { "api_key": "{MY_TEST_KEY}" }
    });
    Config::save(dir.path(), &patch).unwrap();
    let cfg = Config::load(dir.path()).unwrap();
    std::env::remove_var("MY_TEST_KEY");
    assert_eq!(
        cfg.api_key().unwrap(),
        "resolved-secret",
        "{{ENV}} api_key must resolve on reload"
    );
}

#[test]
fn providers_map_resolves_endpoint_by_prefix() {
    let _g = ENV_LOCK.lock().unwrap();
    let (_home_guard, dir) = isolated_home();
    fs::write(
        dir.path().join("opencoder.json"),
        r#"{
            "model": "deepseek/deepseek-chat",
            "providers": {
                "deepseek": {
                    "base_url": "https://api.deepseek.com/v1",
                    "api_key": "sk-deepseek-xxx",
                    "model": "deepseek-chat"
                },
                "openai": {
                    "base_url": "https://api.openai.com/v1",
                    "api_key": "sk-openai-yyy",
                    "model": "gpt-4o"
                }
            }
        }"#,
    )
    .unwrap();
    let cfg = Config::load(dir.path()).unwrap();

    // resolve_endpoint picks the provider matching the model prefix.
    let ep = cfg.resolve_endpoint().unwrap();
    assert_eq!(ep.base_url, "https://api.deepseek.com/v1");
    assert_eq!(ep.api_key, "sk-deepseek-xxx");
}

#[test]
fn providers_map_base_url_for_and_api_key_for() {
    let _g = ENV_LOCK.lock().unwrap();
    let (_home_guard, dir) = isolated_home();
    fs::write(
        dir.path().join("opencoder.json"),
        r#"{
            "model": "deepseek/deepseek-chat",
            "providers": {
                "deepseek": { "base_url": "https://api.deepseek.com/v1", "api_key": "dk-key" },
                "openai": { "base_url": "https://api.openai.com/v1", "api_key": "oai-key" }
            }
        }"#,
    )
    .unwrap();
    let cfg = Config::load(dir.path()).unwrap();

    assert_eq!(cfg.base_url_for("deepseek"), "https://api.deepseek.com/v1");
    assert_eq!(cfg.api_key_for("deepseek").unwrap(), "dk-key");
    assert_eq!(cfg.base_url_for("openai"), "https://api.openai.com/v1");
    assert_eq!(cfg.api_key_for("openai").unwrap(), "oai-key");
    assert!(cfg.provider_for("nonexistent").is_none());
}

#[test]
fn prefix_not_in_providers_falls_back_to_legacy_provider() {
    let _g = ENV_LOCK.lock().unwrap();
    let (_home_guard, dir) = isolated_home();
    fs::write(
        dir.path().join("opencoder.json"),
        r#"{
            "model": "unknown-svc/model-x",
            "provider": { "base_url": "https://legacy.example.com/v1", "api_key": "legacy-key" },
            "providers": {
                "deepseek": { "base_url": "https://api.deepseek.com/v1", "api_key": "dk-key" }
            }
        }"#,
    )
    .unwrap();
    let cfg = Config::load(dir.path()).unwrap();

    // "unknown-svc" is not in providers → fall back to legacy provider field.
    let ep = cfg.resolve_endpoint().unwrap();
    assert_eq!(ep.base_url, "https://legacy.example.com/v1");
    assert_eq!(ep.api_key, "legacy-key");
}

#[test]
fn provider_api_key_missing_falls_back_to_env() {
    let _g = ENV_LOCK.lock().unwrap();
    std::env::set_var("OPENAI_API_KEY", "env-fallback-key");
    let (_home_guard, dir) = isolated_home();
    fs::write(
        dir.path().join("opencoder.json"),
        r#"{
            "model": "deepseek/deepseek-chat",
            "providers": {
                "deepseek": { "base_url": "https://api.deepseek.com/v1" }
            }
        }"#,
    )
    .unwrap();
    let cfg = Config::load(dir.path()).unwrap();

    // No api_key in providers[deepseek], no legacy provider.api_key → env fallback.
    // (api_key_for reads OPENAI_API_KEY live at call time, so keep it set
    // through the resolve, then clean up.)
    let ep = cfg.resolve_endpoint().unwrap();
    assert_eq!(ep.base_url, "https://api.deepseek.com/v1");
    assert_eq!(ep.api_key, "env-fallback-key");
    std::env::remove_var("OPENAI_API_KEY");
}

#[test]
fn merge_into_deep_merges_providers_across_files() {
    let _g = ENV_LOCK.lock().unwrap();
    let (_home_guard, dir) = isolated_home();
    // Simulate two config layers: global provides deepseek base_url, project
    // adds the api_key + a second provider. Both must survive the merge.
    let global = _home_guard;
    let _ = global;
    fs::write(
        dir.path().join("opencoder.json"),
        r#"{
            "providers": {
                "deepseek": { "base_url": "https://api.deepseek.com/v1" },
                "openai": { "base_url": "https://api.openai.com/v1", "api_key": "oai-key" }
            }
        }"#,
    )
    .unwrap();
    // Write a global config that adds deepseek's api_key (merge, not replace).
    let home_dir = std::env::var_os("HOME").unwrap();
    let global_path = std::path::Path::new(&home_dir).join(".opencoder").join("config.json");
    std::fs::create_dir_all(global_path.parent().unwrap()).unwrap();
    std::fs::write(
        &global_path,
        r#"{
            "providers": {
                "deepseek": { "api_key": "dk-key-merged" }
            }
        }"#,
    )
    .unwrap();

    let cfg = Config::load(dir.path()).unwrap();

    // deepseek: base_url from project file, api_key from global file (deep merge).
    assert_eq!(
        cfg.providers.get("deepseek").unwrap().base_url,
        "https://api.deepseek.com/v1"
    );
    assert_eq!(
        cfg.providers.get("deepseek").unwrap().api_key.as_deref(),
        Some("dk-key-merged")
    );
    // openai: only in project file, untouched.
    assert!(cfg.providers.contains_key("openai"));
}

#[test]
fn provider_model_field_round_trips() {
    let _g = ENV_LOCK.lock().unwrap();
    let (_home_guard, dir) = isolated_home();
    fs::write(
        dir.path().join("opencoder.json"),
        r#"{
            "model": "deepseek/deepseek-chat",
            "providers": {
                "deepseek": { "base_url": "https://api.deepseek.com/v1", "model": "deepseek-chat" }
            }
        }"#,
    )
    .unwrap();
    let cfg = Config::load(dir.path()).unwrap();
    assert_eq!(
        cfg.providers.get("deepseek").unwrap().model.as_deref(),
        Some("deepseek-chat")
    );
}

#[test]
fn resolve_endpoint_includes_custom_headers_with_env_resolution() {
    let _g = ENV_LOCK.lock().unwrap();
    std::env::set_var("MY_TENANT", "tenant-42");
    let (_home_guard, dir) = isolated_home();
    fs::write(
        dir.path().join("opencoder.json"),
        r#"{
            "model": "deepseek/deepseek-chat",
            "providers": {
                "deepseek": {
                    "base_url": "https://api.deepseek.com/v1",
                    "api_key": "dk-key",
                    "headers": [
                        { "name": "X-Tenant", "value": "{MY_TENANT}" },
                        { "name": "X-Literal", "value": "static-val" }
                    ]
                }
            }
        }"#,
    )
    .unwrap();
    let cfg = Config::load(dir.path()).unwrap();

    let ep = cfg.resolve_endpoint().unwrap();
    assert_eq!(ep.base_url, "https://api.deepseek.com/v1");
    assert_eq!(ep.api_key, "dk-key");
    assert_eq!(ep.headers.len(), 2);
    // {MY_TENANT} env reference is resolved at endpoint-resolution time.
    assert_eq!(ep.headers[0], ("X-Tenant".to_string(), "tenant-42".to_string()));
    // A literal value passes through unchanged.
    assert_eq!(ep.headers[1], ("X-Literal".to_string(), "static-val".to_string()));
    std::env::remove_var("MY_TENANT");
}

/// Isolate HOME + XDG_CONFIG_HOME into a temp dir so `Config::load` from `dir`
/// does not pick up the developer's real global config. Returns the home guard
/// (keep it alive for the test body) and a clean working-dir tempdir.
fn isolated_home() -> (HomeGuard, tempfile::TempDir) {
    let home = tempfile::tempdir().unwrap();
    let prev_home = std::env::var_os("HOME");
    let prev_xdg = std::env::var_os("XDG_CONFIG_HOME");
    std::env::set_var("HOME", home.path());
    std::env::set_var("XDG_CONFIG_HOME", home.path());
    let cwd = tempfile::tempdir().unwrap();
    (
        HomeGuard {
            prev_home,
            prev_xdg,
        },
        cwd,
    )
}

struct HomeGuard {
    prev_home: Option<std::ffi::OsString>,
    prev_xdg: Option<std::ffi::OsString>,
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        match &self.prev_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        match &self.prev_xdg {
            Some(h) => std::env::set_var("XDG_CONFIG_HOME", h),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }
}
