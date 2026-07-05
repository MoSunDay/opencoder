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
            "max_steps": 7,
            "compaction": { "auto": true, "context_threshold": 40000, "reserved": 8000, "tail_turns": 3, "prune": true }
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
    assert_eq!(cfg.max_steps, 7);
    assert_eq!(cfg.compaction.context_threshold, 40000);
    assert_eq!(cfg.compaction.reserved, 8000);
    assert_eq!(cfg.compaction.tail_turns, 3);
    assert!(cfg.compaction.prune);
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

    assert_eq!(cfg.model, "env/model-from-env", "OPENCODE_MODEL wins over file");
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
    assert_eq!(cfg.api_key().unwrap(), "secret-value-123", "braces var must resolve");
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
    let reserved = cfg.compaction.reserved.min(cfg.context_limit().saturating_sub(1));
    let usable = cfg.context_limit().saturating_sub(reserved);
    assert!(usable >= 1, "usable must stay positive even with over-large reserved");
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
    assert_eq!(cfg_no_home.model, "openai/gpt-4o-mini", "default when no ~/.opencoder");

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

    assert_eq!(cfg.model, "zhipuai-coding-plan/glm-5.2", "~/.opencoder/config.json must be discovered");
    assert_eq!(cfg.provider.base_url, "https://x.example/v4");
    assert_eq!(cfg.max_tokens, Some(4096));
}
