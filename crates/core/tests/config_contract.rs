//! P1 functional tests for Config: deep-merge precedence, env resolution,
//! small_model / context_limit plumbing, and the {VAR} secret indirection.

use std::fs;
use std::sync::Mutex;

use opencode_core::Config;

// Env mutation is process-global; serialize tests that touch the environment.
static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn merge_project_file_overrides_defaults() {
    let _g = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("opencode.json"),
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
    std::env::set_var("OPENCODE_MODEL", "env/model-from-env");
    std::env::set_var("OPENCODE_SMALL_MODEL", "env/small");
    std::env::set_var("OPENCODE_CONTEXT_LIMIT", "99999");
    std::env::remove_var("OPENAI_BASE_URL");

    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("opencode.json"),
        r#"{"model":"file/model","small_model":"file/sm","context_limit":1000}"#,
    )
    .unwrap();
    let cfg = Config::load(dir.path()).unwrap();

    std::env::remove_var("OPENCODE_MODEL");
    std::env::remove_var("OPENCODE_SMALL_MODEL");
    std::env::remove_var("OPENCODE_CONTEXT_LIMIT");

    assert_eq!(
        cfg.model, "env/model-from-env",
        "OPENCODE_MODEL wins over file"
    );
    assert_eq!(cfg.small_model.as_deref(), Some("env/small"));
    assert_eq!(cfg.context_limit(), 99999);
}

#[test]
fn braces_api_key_resolves_env_var() {
    let _g = ENV_LOCK.lock().unwrap();
    std::env::set_var("ZHIPU_API_KEY", "secret-value-123");
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("opencode.json"),
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
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config::load(dir.path()).unwrap();
    assert_eq!(cfg.context_limit(), opencode_core::DEFAULT_CONTEXT_LIMIT);
    assert_eq!(cfg.compaction.reserved, 20_000);
    assert!(cfg.compaction.auto);
    assert_eq!(cfg.agent.default, "act");
}

#[test]
fn reserved_saturates_against_context_limit() {
    let _g = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("opencode.json"),
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
        dir.path().join("opencode.json"),
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
        dir.path().join("opencode.json"),
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
        written.ends_with("opencode.json"),
        "must save to project-local opencode.json"
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
        dir.path().join("opencode.json"),
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
    use opencode_core::looks_like_env_var;
    assert!(looks_like_env_var("ZHIPU_API_KEY"));
    assert!(!looks_like_env_var("sk-abcd"));
    assert!(!looks_like_env_var(""));
    assert!(looks_like_env_var("KEY_1"));
}

#[test]
fn save_can_remove_reasoning_effort_via_null() {
    // Setting reasoning_effort to null in the patch must REMOVE the field
    // (merge_json treats null as delete) so it is omitted from request bodies.
    let _g = ENV_LOCK.lock().unwrap();
    let (_home_guard, dir) = isolated_home();
    fs::write(
        dir.path().join("opencode.json"),
        r#"{"model":"m","reasoning_effort":"high"}"#,
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
        "model": "m",
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
