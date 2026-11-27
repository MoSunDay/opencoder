use opencoder_core::{
    activate_config_env, config_env_status, create_config_env, deactivate_config_env,
    delete_config_env, list_config_envs, scoped_config_home, validate_config_env_name, Config,
    ConfigEnvInfo, ConfigMode,
};
use serde_json::Value;
use std::path::Path;

fn write_json(path: &Path, value: Value) {
    std::fs::write(path, value.to_string()).unwrap();
}

fn populate(env: &ConfigEnvInfo, label: &str) {
    write_json(
        &env.path.join("config.json"),
        serde_json::json!({"model": format!("{label}-provider/{label}-model")}),
    );
    write_json(
        &env.path.join("cli.json"),
        serde_json::json!({format!("{label}-cli"): {"enabled": true, "content": label}}),
    );
    write_json(
        &env.path.join("skills.json"),
        serde_json::json!({format!("{label}-skill"): {"enabled": true}}),
    );
}

fn assert_view(work: &Path, label: &str) {
    let config = Config::load(work).unwrap();
    assert_eq!(config.model, format!("{label}-provider/{label}-model"));
    assert_eq!(config.enabled_cli()[0].0, format!("{label}-cli"));
    assert_eq!(config.enabled_skill_names(), vec![format!("{label}-skill")]);
}

#[test]
fn switching_a_b_a_reloads_each_env_config_cli_and_skills() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _guard = scoped_config_home(home.path().to_path_buf());
    let a = create_config_env("a").unwrap();
    let b = create_config_env("b").unwrap();
    populate(&a, "a");
    populate(&b, "b");

    activate_config_env("a").unwrap();
    assert_view(work.path(), "a");
    activate_config_env("b").unwrap();
    assert_view(work.path(), "b");
    activate_config_env("a").unwrap();
    assert_view(work.path(), "a");

    let listed = list_config_envs().unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|env| env.name.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert!(listed[0].active);
    assert!(!listed[1].active);
    delete_config_env("a").unwrap();
    assert_eq!(config_env_status().unwrap().active, None);
    assert!(!a.path.exists());
    deactivate_config_env().unwrap();
    delete_config_env("b").unwrap();
    assert!(list_config_envs().unwrap().is_empty());
}

#[test]
fn malformed_activation_preserves_the_original_marker_and_view() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _guard = scoped_config_home(home.path().to_path_buf());
    let a = create_config_env("a").unwrap();
    let b = create_config_env("b").unwrap();
    populate(&a, "a");
    populate(&b, "b");
    activate_config_env("a").unwrap();
    let marker = config_env_status().unwrap().marker_path;
    let original = std::fs::read(&marker).unwrap();

    std::fs::write(b.path.join("config.json"), b"{not json").unwrap();
    assert!(activate_config_env("b").is_err());
    assert_eq!(std::fs::read(&marker).unwrap(), original);
    assert_eq!(config_env_status().unwrap().active.as_deref(), Some("a"));
    assert_view(work.path(), "a");

    populate(&b, "b");
    write_json(
        &b.path.join("config.json"),
        serde_json::json!({"modes": {"act": {"model": "m/g"}}}),
    );
    assert!(activate_config_env("b").is_err());
    assert_eq!(std::fs::read(&marker).unwrap(), original);

    populate(&b, "b");
    write_json(
        &b.path.join("skills.json"),
        serde_json::json!({"bad": "not-an-object"}),
    );
    assert!(activate_config_env("b").is_err());
    assert_eq!(std::fs::read(&marker).unwrap(), original);

    populate(&b, "b");
    std::fs::write(b.path.join("mcp.json"), b"{not json").unwrap();
    std::fs::write(b.path.join("schedules.json"), b"not json").unwrap();
    activate_config_env("b").unwrap();
    assert_view(work.path(), "b");
}

#[test]
fn null_modes_preflight_resets_merged_modes_without_accepting_invalid_config() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _guard = scoped_config_home(home.path().to_path_buf());
    let env = create_config_env("env").unwrap();
    write_json(
        &home.path().join(".opencoder/config.json"),
        serde_json::json!({
            "model": "legacy/base-model",
            "max_tokens": 100,
            "modes": {
                "shared": {"model": "shared/shared-model", "max_tokens": 200},
                "act": {"model": "act/act-model", "max_tokens": 400}
            }
        }),
    );
    write_json(
        &env.path.join("config.json"),
        serde_json::json!({"modes": null}),
    );

    activate_config_env("env").unwrap();
    let loaded = Config::load(work.path()).unwrap();
    assert!(loaded.modes.is_empty());
    let act = loaded.effective_for_mode(ConfigMode::Act);
    assert_eq!(act.model, "legacy/base-model");
    assert_eq!(act.max_tokens, Some(100));

    let marker = config_env_status().unwrap().marker_path;
    let original = std::fs::read(&marker).unwrap();
    for invalid in [
        serde_json::json!({"modes": "invalid"}),
        serde_json::json!({"modes": {"act": "invalid"}}),
        serde_json::json!({"modes": null, "max_tokens": "invalid"}),
    ] {
        write_json(&env.path.join("config.json"), invalid.clone());
        let attempted = activate_config_env("env");
        assert!(attempted.is_err(), "accepted invalid config {invalid}");
        assert_eq!(std::fs::read(&marker).unwrap(), original);
    }
}

#[test]
fn stale_marker_is_ignored_and_can_be_replaced_by_activation() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _guard = scoped_config_home(home.path().to_path_buf());
    let global = home.path().join(".opencoder/config.json");
    std::fs::create_dir_all(global.parent().unwrap()).unwrap();
    write_json(&global, serde_json::json!({"model": "global/model"}));
    let status = config_env_status().unwrap();
    std::fs::write(&status.marker_path, "missing\n").unwrap();

    let status = config_env_status().unwrap();
    assert!(status.stale_marker);
    assert_eq!(status.active, None);
    assert_eq!(Config::load(work.path()).unwrap().model, "global/model");

    let env = create_config_env("valid").unwrap();
    populate(&env, "valid");
    let activated = activate_config_env("valid").unwrap();
    assert_eq!(activated.active.as_deref(), Some("valid"));
    assert!(!activated.stale_marker);
    assert_view(work.path(), "valid");
}

#[test]
fn env_names_are_validated_before_any_filesystem_operation() {
    let home = tempfile::tempdir().unwrap();
    let _guard = scoped_config_home(home.path().to_path_buf());
    for name in [
        "",
        "../escape",
        "a/b",
        ".hidden",
        "white space",
        "bad\0name",
    ] {
        assert!(validate_config_env_name(name).is_err(), "accepted {name:?}");
        assert!(create_config_env(name).is_err(), "created {name:?}");
    }
    assert!(validate_config_env_name("valid_name-2").is_ok());
    assert!(delete_config_env("missing").is_err());
    assert!(!home.path().join(".opencoder/envs").exists());
}

#[cfg(unix)]
#[test]
fn env_config_files_and_active_marker_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let home = tempfile::tempdir().unwrap();
    let _guard = scoped_config_home(home.path().to_path_buf());
    let env = create_config_env("private").unwrap();
    for file in ["config.json", "cli.json", "skills.json"] {
        let mode = std::fs::metadata(env.path.join(file))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "{file}");
    }
    let status = activate_config_env("private").unwrap();
    let marker_mode = std::fs::metadata(status.marker_path)
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(marker_mode & 0o777, 0o600);
    let directory_mode = std::fs::metadata(&env.path).unwrap().permissions().mode();
    assert_eq!(directory_mode & 0o777, 0o700);
}
