use opencoder_core::{
    activate_config_env, config_env_status, create_config_env, deactivate_config_env,
    scoped_config_home, Config, ConfigSaveRoute,
};
use serde_json::{json, Value};
use std::path::Path;

fn write_json(path: &Path, value: Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, value.to_string()).unwrap();
}

fn cli_value(label: &str) -> Value {
    json!({format!("{label}-cli"): {"enabled": true, "content": label}})
}

fn assert_model_and_cli(config: &Config, label: &str) {
    assert_eq!(config.model, format!("{label}/{label}-model"));
    assert_eq!(config.enabled_cli()[0].0, format!("{label}-cli"));
}

#[test]
fn active_env_project_global_and_xdg_have_strict_precedence() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _guard = scoped_config_home(home.path().to_path_buf());
    let xdg_config = home.path().join("opencoder/config.json");
    let xdg_cli = home.path().join("opencoder/cli.json");
    let global_config = home.path().join(".opencoder/config.json");
    let global_cli = home.path().join(".opencoder/cli.json");
    let project_config = work.path().join("opencoder.json");
    let project_cli = work.path().join(".opencoder/cli.json");
    for (path, label) in [
        (&xdg_config, "xdg"),
        (&global_config, "global"),
        (&project_config, "project"),
    ] {
        write_json(path, json!({"model": format!("{label}/{label}-model")}));
    }
    for (path, label) in [
        (&xdg_cli, "xdg"),
        (&global_cli, "global"),
        (&project_cli, "project"),
    ] {
        write_json(path, cli_value(label));
    }
    let env = create_config_env("env").unwrap();
    write_json(
        &env.path.join("config.json"),
        json!({"model": "env/env-model"}),
    );
    write_json(&env.path.join("cli.json"), cli_value("env"));
    activate_config_env("env").unwrap();

    assert_model_and_cli(&Config::load(work.path()).unwrap(), "env");
    deactivate_config_env().unwrap();
    assert_model_and_cli(&Config::load(work.path()).unwrap(), "project");
    std::fs::remove_file(&project_config).unwrap();
    std::fs::remove_file(&project_cli).unwrap();
    assert_model_and_cli(&Config::load(work.path()).unwrap(), "global");
    std::fs::remove_file(&global_config).unwrap();
    std::fs::remove_file(&global_cli).unwrap();
    assert_model_and_cli(&Config::load(work.path()).unwrap(), "xdg");
}

#[test]
fn scoped_frozen_and_operator_loaders_ignore_interactive_active_env() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let operator = tempfile::tempdir().unwrap();
    let _guard = scoped_config_home(home.path().to_path_buf());
    write_json(
        &home.path().join(".opencoder/config.json"),
        json!({"model": "global/global-model"}),
    );
    write_json(
        &home.path().join(".opencoder/cli.json"),
        cli_value("global"),
    );
    let env = create_config_env("env").unwrap();
    write_json(
        &env.path.join("config.json"),
        json!({"model": "env/env-model"}),
    );
    write_json(&env.path.join("cli.json"), cli_value("env"));
    activate_config_env("env").unwrap();
    assert_model_and_cli(&Config::load(work.path()).unwrap(), "env");

    let scoped = Config::load_with_home(work.path(), Some(home.path())).unwrap();
    assert_model_and_cli(&scoped, "global");
    let frozen = Config::load_with_home_frozen(work.path(), None).unwrap();
    assert_model_and_cli(&frozen, "global");

    write_json(
        &operator.path().join("config.json"),
        json!({"model": "operator/operator-model"}),
    );
    write_json(&operator.path().join("cli.json"), cli_value("operator"));
    let operator = Config::load_operator(operator.path()).unwrap();
    assert_model_and_cli(&operator, "operator");
}

#[test]
fn env_mcp_and_schedules_do_not_override_legacy_domain_sources() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _guard = scoped_config_home(home.path().to_path_buf());
    let env = create_config_env("env").unwrap();
    write_json(
        &env.path.join("mcp.json"),
        json!({"env": {"enabled": true}}),
    );
    write_json(
        &env.path.join("schedules.json"),
        json!({"scan_interval_secs": 10}),
    );
    write_json(
        &home.path().join("opencoder/mcp.json"),
        json!({"xdg": {"enabled": true}}),
    );
    write_json(
        &home.path().join("opencoder/schedules.json"),
        json!({"scan_interval_secs": 30}),
    );
    activate_config_env("env").unwrap();

    let config = Config::load(work.path()).unwrap();
    assert!(config.mcp_servers.is_empty());
    assert_eq!(config.schedules.scan_interval_secs, None);

    write_json(
        &home.path().join(".opencoder/mcp.json"),
        json!({"global": {"enabled": true}}),
    );
    write_json(
        &home.path().join(".opencoder/schedules.json"),
        json!({"scan_interval_secs": 20}),
    );
    let config = Config::load(work.path()).unwrap();
    assert_eq!(config.mcp_servers.keys().collect::<Vec<_>>(), ["global"]);
    assert_eq!(config.schedules.scan_interval_secs, Some(20));

    write_json(
        &work.path().join(".opencoder/mcp.json"),
        json!({"project": {"enabled": true}}),
    );
    write_json(
        &work.path().join(".opencoder/schedules.json"),
        json!({"scan_interval_secs": 40}),
    );
    let config = Config::load(work.path()).unwrap();
    assert_eq!(config.mcp_servers.keys().collect::<Vec<_>>(), ["project"]);
    assert_eq!(config.schedules.scan_interval_secs, Some(40));
}

#[test]
fn env_aware_and_legacy_save_routes_do_not_leak_into_each_other() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _guard = scoped_config_home(home.path().to_path_buf());
    let env = create_config_env("env").unwrap();
    activate_config_env("env").unwrap();

    let automatic = Config::save_target_for(work.path(), ConfigSaveRoute::Auto).unwrap();
    assert_eq!(automatic.route, ConfigSaveRoute::Project);
    assert_eq!(automatic.active_env, None);
    assert_eq!(automatic.path, work.path().join("opencoder.json"));
    let env_aware = Config::save_target_for(work.path(), ConfigSaveRoute::EnvAware).unwrap();
    assert_eq!(env_aware.route, ConfigSaveRoute::ActiveEnv);
    assert_eq!(env_aware.active_env.as_deref(), Some("env"));
    assert_eq!(env_aware.path, env.path.join("config.json"));
    assert_eq!(
        Config::save_target_for(work.path(), ConfigSaveRoute::Project)
            .unwrap()
            .path,
        work.path().join("opencoder.json")
    );

    Config::save_with_route(
        work.path(),
        &json!({"model": "env/env-model", "modes": {"act": {"max_tokens": 8192}}}),
        ConfigSaveRoute::EnvAware,
    )
    .unwrap();
    Config::save(work.path(), &json!({"keymap": {"help": "alt+h"}})).unwrap();
    Config::save_with_route(
        work.path(),
        &json!({"model": "project/project-model"}),
        ConfigSaveRoute::Project,
    )
    .unwrap();
    Config::save_with_route(
        work.path(),
        &json!({"model": "global/global-model"}),
        ConfigSaveRoute::Global,
    )
    .unwrap();
    Config::save_with_route(
        work.path(),
        &json!({"cli": cli_value("env")}),
        ConfigSaveRoute::EnvAware,
    )
    .unwrap();
    Config::save_with_route(
        work.path(),
        &json!({"skills": {"env-skill": {"enabled": true}}}),
        ConfigSaveRoute::EnvAware,
    )
    .unwrap();
    Config::save(work.path(), &json!({"cli": cli_value("legacy")})).unwrap();
    Config::save(
        work.path(),
        &json!({"mcp_servers": {"legacy": {"enabled": true}}}),
    )
    .unwrap();
    Config::save_with_route(
        work.path(),
        &json!({"cli": cli_value("project")}),
        ConfigSaveRoute::Project,
    )
    .unwrap();
    Config::save_with_route(
        work.path(),
        &json!({"cli": cli_value("global")}),
        ConfigSaveRoute::Global,
    )
    .unwrap();

    let read = |path: &Path| -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    };
    assert_eq!(
        read(&env.path.join("config.json"))["model"],
        "env/env-model"
    );
    assert_eq!(
        read(&env.path.join("config.json"))["modes"]["act"]["max_tokens"],
        8192
    );
    assert!(read(&env.path.join("config.json")).get("keymap").is_none());
    assert_eq!(
        read(&work.path().join("opencoder.json"))["model"],
        "project/project-model"
    );
    assert_eq!(
        read(&work.path().join("opencoder.json"))["keymap"]["help"],
        "alt+h"
    );
    assert_eq!(
        read(&home.path().join(".opencoder/config.json"))["model"],
        "global/global-model"
    );
    assert!(read(&env.path.join("cli.json"))["env-cli"].is_object());
    assert!(read(&env.path.join("skills.json"))["env-skill"].is_object());
    assert!(read(&work.path().join(".opencoder/cli.json"))["project-cli"].is_object());
    assert!(read(&home.path().join(".opencoder/cli.json"))["global-cli"].is_object());
    assert!(read(&home.path().join(".opencoder/cli.json"))["legacy-cli"].is_object());
    assert!(read(&home.path().join(".opencoder/mcp.json"))["legacy"].is_object());
    assert!(!env.path.join("mcp.json").exists());

    deactivate_config_env().unwrap();
    assert!(Config::save_target_for(work.path(), ConfigSaveRoute::ActiveEnv).is_err());
    let automatic = Config::save_target_for(work.path(), ConfigSaveRoute::Auto).unwrap();
    let fallback = Config::save_target_for(work.path(), ConfigSaveRoute::EnvAware).unwrap();
    assert_eq!(fallback, automatic);
    Config::save_with_route(
        work.path(),
        &json!({"model": "fallback/fallback-model"}),
        ConfigSaveRoute::EnvAware,
    )
    .unwrap();
    assert_eq!(
        read(&work.path().join("opencoder.json"))["model"],
        "fallback/fallback-model"
    );
}

#[test]
fn live_snapshot_returns_raw_domains_from_shared_env_and_legacy_routes() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _guard = scoped_config_home(home.path().to_path_buf());
    let env = create_config_env("snapshot").unwrap();
    write_json(
        &env.path.join("config.json"),
        json!({"model": "env/env-model"}),
    );
    write_json(&env.path.join("cli.json"), cli_value("env"));
    write_json(
        &env.path.join("skills.json"),
        json!({"env-skill": {"enabled": true}}),
    );
    write_json(
        &env.path.join("mcp.json"),
        json!({"wrong-env-mcp": {"command": "wrong"}}),
    );
    write_json(
        &work.path().join(".opencoder/mcp.json"),
        json!({"project-mcp": {"command": "project"}}),
    );
    write_json(
        &work.path().join(".opencoder/schedules.json"),
        json!({"scan_interval_secs": 17}),
    );
    activate_config_env("snapshot").unwrap();

    let (config, values) = Config::load_with_domain_values(
        work.path(),
        &["cli", "mcp_servers", "schedules", "skills", "unknown"],
    )
    .unwrap();
    assert_model_and_cli(&config, "env");
    assert!(config.mcp_servers.contains_key("project-mcp"));
    assert!(!config.mcp_servers.contains_key("wrong-env-mcp"));
    assert_eq!(config.schedules.scan_interval_secs, Some(17));
    assert_eq!(values[0], Some(cli_value("env")));
    assert!(values[1].as_ref().unwrap()["project-mcp"].is_object());
    assert_eq!(values[2].as_ref().unwrap()["scan_interval_secs"], 17);
    assert!(values[3].as_ref().unwrap()["env-skill"].is_object());
    assert_eq!(values[4], None);
}

#[test]
fn unreadable_marker_fails_closed_while_stale_marker_keeps_save_fallback() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _guard = scoped_config_home(home.path().to_path_buf());
    let project = work.path().join("opencoder.json");
    let global = home.path().join(".opencoder/config.json");
    let project_cli = work.path().join(".opencoder/cli.json");
    let global_cli = home.path().join(".opencoder/cli.json");
    write_json(&project, json!({"model": "project/project-model"}));
    write_json(&global, json!({"model": "global/global-model"}));
    write_json(&project_cli, cli_value("project"));
    write_json(&global_cli, cli_value("global"));
    let original: Vec<Vec<u8>> = [&project, &global, &project_cli, &global_cli]
        .into_iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect();

    // A directory at the marker path produces a deterministic non-NotFound
    // read error even for privileged test users.
    let marker = config_env_status().unwrap().marker_path;
    std::fs::create_dir(&marker).unwrap();
    assert!(Config::load(work.path()).is_err());
    assert!(Config::load_with_domain_values(work.path(), &["mcp_servers"]).is_err());
    assert!(Config::save_target_for(work.path(), ConfigSaveRoute::EnvAware).is_err());
    for patch in [
        json!({"model": "fallback/should-not-write"}),
        json!({"cli": cli_value("should-not-write")}),
        json!({"mcp_servers": {"should-not-write": {"enabled": true}}}),
    ] {
        assert!(Config::save_with_route(work.path(), &patch, ConfigSaveRoute::EnvAware).is_err());
    }
    for (path, before) in [&project, &global, &project_cli, &global_cli]
        .into_iter()
        .zip(original)
    {
        assert_eq!(std::fs::read(path).unwrap(), before, "{}", path.display());
    }

    std::fs::remove_dir(&marker).unwrap();
    std::fs::write(&marker, "missing\n").unwrap();
    assert_eq!(
        Config::load(work.path()).unwrap().model,
        "project/project-model"
    );
    let fallback = Config::save_target_for(work.path(), ConfigSaveRoute::EnvAware).unwrap();
    assert_eq!(fallback.route, ConfigSaveRoute::Project);
    Config::save_with_route(
        work.path(),
        &json!({"model": "project/stale-fallback", "cli": cli_value("stale")}),
        ConfigSaveRoute::EnvAware,
    )
    .unwrap();
    let read = |path: &Path| -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    };
    assert_eq!(read(&project)["model"], "project/stale-fallback");
    assert!(read(&project_cli)["stale-cli"].is_object());
    assert_eq!(read(&global)["model"], "global/global-model");
    assert!(read(&global_cli)["stale-cli"].is_null());
}
