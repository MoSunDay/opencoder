//! Provider-map and global-config contract tests for Config: endpoint
//! resolution by provider prefix, deep-merge of providers across files,
//! custom headers with env resolution, and save / ensure-global semantics.

use std::fs;
use std::sync::Mutex;

use opencoder_core::Config;

// Env mutation is process-global; serialize tests that touch the environment.
static ENV_LOCK: Mutex<()> = Mutex::new(());

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

/// The missing-key error must be actionable: name the provider and list all
/// three configuration avenues (registry entry, top-level provider, env var).
#[test]
fn api_key_error_names_provider_and_all_config_avenues() {
    let _g = ENV_LOCK.lock().unwrap();
    std::env::remove_var("OPENAI_API_KEY");
    let (_home_guard, dir) = isolated_home();
    fs::write(
        dir.path().join("opencoder.json"),
        r#"{
            "model": "zhipuai-coding-plan/glm-5.2",
            "providers": {
                "zhipuai-coding-plan": { "base_url": "https://bigmodel.cn/v4" }
            }
        }"#,
    )
    .unwrap();
    let cfg = Config::load(dir.path()).unwrap();

    let err = cfg.api_key_for("zhipuai-coding-plan").unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("zhipuai-coding-plan"),
        "error must name the provider; got: {msg}"
    );
    assert!(
        msg.contains("OPENAI_API_KEY"),
        "error must mention the env var; got: {msg}"
    );
    assert!(
        msg.contains("providers.zhipuai-coding-plan.api_key")
            && msg.contains("provider.api_key"),
        "error must mention the registry entry and top-level provider keys; got: {msg}"
    );
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

/// Bug fix: `OPENAI_BASE_URL` must sync the *active* provider registry entry.
/// `OPENCODER_MODEL` can switch the active provider (via its `provider/model`
/// prefix); applying the env base_url only to the legacy top-level
/// `provider.base_url` left the registry entry's file-level base_url stale,
/// silently ignoring the env override at endpoint resolution.
#[test]
fn openai_base_url_env_overrides_active_provider_registry_entry() {
    let _g = ENV_LOCK.lock().unwrap();
    std::env::set_var("OPENCODER_MODEL", "zhipuai-coding-plan/glm-5.2");
    std::env::set_var("OPENAI_BASE_URL", "https://env.example/v1");
    let (_home_guard, dir) = isolated_home();
    fs::write(
        dir.path().join("opencoder.json"),
        r#"{
            "model": "deepseek/deepseek-chat",
            "providers": {
                "zhipuai-coding-plan": {
                    "base_url": "https://old-registry.example/v4",
                    "api_key": "zk"
                },
                "deepseek": { "base_url": "https://api.deepseek.com/v1" }
            }
        }"#,
    )
    .unwrap();
    let cfg = Config::load(dir.path()).unwrap();
    // Env overlay is applied at load; clean up before asserting.
    std::env::remove_var("OPENCODER_MODEL");
    std::env::remove_var("OPENAI_BASE_URL");

    assert_eq!(cfg.model, "zhipuai-coding-plan/glm-5.2");
    // The registry entry for the now-active provider picks up the env value…
    assert_eq!(
        cfg.providers["zhipuai-coding-plan"].base_url,
        "https://env.example/v1",
        "active provider registry entry must sync OPENAI_BASE_URL"
    );
    // …and the legacy top-level provider field keeps the old behavior…
    assert_eq!(cfg.provider.base_url, "https://env.example/v1");
    // …while a non-active provider's entry stays untouched.
    assert_eq!(
        cfg.providers["deepseek"].base_url,
        "https://api.deepseek.com/v1"
    );
    // Endpoint resolution (what ChatClient actually uses) sees the env value.
    let ep = cfg.resolve_endpoint().unwrap();
    assert_eq!(ep.base_url, "https://env.example/v1");
}

/// Second case: the active provider has NO registry entry — only the legacy
/// top-level `provider.base_url` is written, and nothing panics.
#[test]
fn openai_base_url_env_without_registry_entry_updates_legacy_only() {
    let _g = ENV_LOCK.lock().unwrap();
    std::env::set_var("OPENCODER_MODEL", "unknown-svc/model-x");
    std::env::set_var("OPENAI_BASE_URL", "https://env.example/v1");
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
    std::env::remove_var("OPENCODER_MODEL");
    std::env::remove_var("OPENAI_BASE_URL");

    assert_eq!(
        cfg.provider.base_url, "https://env.example/v1",
        "legacy top-level base_url still takes the env override"
    );
    // No entry for "unknown-svc": the untouched provider map must not panic
    // (previously a `.get_mut` on a missing key silently no-oped anyway, but
    // this pins the contract).
    assert!(!cfg.providers.contains_key("unknown-svc"));
    assert_eq!(
        cfg.providers["deepseek"].base_url,
        "https://api.deepseek.com/v1",
        "non-active registry entries stay at their file-level values"
    );
}

/// Trailing-slash normalization applies to the registry sync too.
#[test]
fn openai_base_url_env_registry_sync_normalizes_trailing_slash() {
    let _g = ENV_LOCK.lock().unwrap();
    std::env::set_var("OPENCODER_MODEL", "zhipuai-coding-plan/glm-5.2");
    std::env::set_var("OPENAI_BASE_URL", "https://env.example/v1/");
    let (_home_guard, dir) = isolated_home();
    fs::write(
        dir.path().join("opencoder.json"),
        r#"{
            "providers": {
                "zhipuai-coding-plan": { "base_url": "https://old.example/v4" }
            }
        }"#,
    )
    .unwrap();
    let cfg = Config::load(dir.path()).unwrap();
    std::env::remove_var("OPENCODER_MODEL");
    std::env::remove_var("OPENAI_BASE_URL");

    assert_eq!(cfg.providers["zhipuai-coding-plan"].base_url, "https://env.example/v1");
    assert_eq!(cfg.provider.base_url, "https://env.example/v1");
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
    let global_path = std::path::Path::new(&home_dir)
        .join(".opencoder")
        .join("config.json");
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
    assert_eq!(
        ep.headers[0],
        ("X-Tenant".to_string(), "tenant-42".to_string())
    );
    // A literal value passes through unchanged.
    assert_eq!(
        ep.headers[1],
        ("X-Literal".to_string(), "static-val".to_string())
    );
    std::env::remove_var("MY_TENANT");
}

#[test]
fn merge_handles_output_streamline_tool_guard_subagent_drain() {
    let _g = ENV_LOCK.lock().unwrap();
    let (_home_guard, dir) = isolated_home();
    fs::write(
        dir.path().join("opencoder.json"),
        r#"{
            "output_streamline": {
                "enabled": false,
                "trim_trailing": false,
                "collapse_blank_lines": false,
                "trim_outer": false,
                "collapse_inline_ws": true
            },
            "tool_guard": {
                "max_consecutive_failures": 7,
                "backoff_base_ms": 500,
                "backoff_max_ms": 4000
            },
            "subagent_drain_secs": 42
        }"#,
    )
    .unwrap();

    let cfg = Config::load(dir.path()).unwrap();

    // output_streamline section is no longer silently ignored.
    assert!(!cfg.output_streamline.enabled);
    assert!(!cfg.output_streamline.trim_trailing);
    assert!(!cfg.output_streamline.collapse_blank_lines);
    assert!(!cfg.output_streamline.trim_outer);
    assert!(cfg.output_streamline.collapse_inline_ws);

    // tool_guard section is no longer silently ignored.
    assert_eq!(cfg.tool_guard.max_consecutive_failures, 7);
    assert_eq!(cfg.tool_guard.backoff_base_ms, 500);
    assert_eq!(cfg.tool_guard.backoff_max_ms, 4000);

    // subagent_drain_secs is no longer silently ignored.
    assert_eq!(cfg.subagent_drain_secs, Some(42));
}

#[test]
fn default_provider_base_url_is_openai() {
    // Config::default() must agree with the serde default for base_url.
    let cfg = Config::default();
    assert_eq!(cfg.provider.base_url, "https://api.openai.com/v1");
}

#[test]
fn ensure_global_config_creates_empty_file_without_clobbering() {
    let home = tempfile::tempdir().unwrap();
    let _isolation = opencoder_core::scoped_config_home(home.path().to_path_buf());

    let (path, created) = Config::ensure_global_config().unwrap();
    assert!(created);
    assert_eq!(path, home.path().join(".opencoder/config.json"));
    assert_eq!(fs::read_to_string(&path).unwrap(), "{}\n");

    fs::write(&path, r#"{"keep":true}"#).unwrap();
    let (same_path, created_again) = Config::ensure_global_config().unwrap();
    assert_eq!(same_path, path);
    assert!(!created_again);
    assert_eq!(fs::read_to_string(path).unwrap(), r#"{"keep":true}"#);
}

#[cfg(unix)]
#[test]
fn ensure_global_config_is_private_on_unix() {
    use std::os::unix::fs::PermissionsExt;

    let home = tempfile::tempdir().unwrap();
    let _isolation = opencoder_core::scoped_config_home(home.path().to_path_buf());
    let (path, _) = Config::ensure_global_config().unwrap();
    let mode = fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn save_global_ignores_project_save_target_and_preserves_unknown_keys() {
    let home = tempfile::tempdir().unwrap();
    let _isolation = opencoder_core::scoped_config_home(home.path().to_path_buf());
    let workdir = tempfile::tempdir().unwrap();
    let project = workdir.path().join("opencoder.json");
    fs::write(&project, r#"{"model":"project/model"}"#).unwrap();
    let global = home.path().join(".opencoder/config.json");
    fs::create_dir_all(global.parent().unwrap()).unwrap();
    fs::write(&global, r#"{"keep":true}"#).unwrap();

    let written = Config::save_global(&serde_json::json!({
        "model": "demo/model",
        "providers": {"demo": {"base_url": "https://example.com/v1", "api_key": "secret"}}
    }))
    .unwrap();

    assert_eq!(written, global);
    assert_eq!(
        fs::read_to_string(project).unwrap(),
        r#"{"model":"project/model"}"#
    );
    let saved: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(global).unwrap()).unwrap();
    assert_eq!(saved["keep"], true);
    assert_eq!(saved["model"], "demo/model");
}

#[test]
fn merged_with_is_pure_and_uses_config_merge_rules() {
    let original = Config::default();
    let merged = original.merged_with(&serde_json::json!({
        "model": "demo/model",
        "providers": {"demo": {"base_url": "https://example.com/v1", "api_key": "key"}}
    }));

    assert_eq!(original.model, "openai/gpt-4o-mini");
    assert!(original.providers.is_empty());
    assert_eq!(merged.model, "demo/model");
    assert_eq!(merged.providers["demo"].api_key.as_deref(), Some("key"));
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
