use std::path::Path;

use crate::command::EnvCommand;
use crate::model_menu::env::{handle_key, EnvMenu};
use crate::model_menu::{ModeMenuKind, ModeSelection, ModelMenu, ModelOutcome};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::{
    activate_config_env, config_env_status, create_config_env, Config, ConfigMode, ConfigSaveRoute,
};

fn execute(workdir: &Path, command: EnvCommand) -> (ModelOutcome, Option<ModelMenu>) {
    handle_key(
        EnvMenu::with_command(workdir, command),
        KeyEvent::new(KeyCode::Null, KeyModifiers::NONE),
    )
}

fn write_env_config(home: &Path, name: &str, value: serde_json::Value) {
    std::fs::write(
        home.join(".opencoder/envs").join(name).join("config.json"),
        value.to_string(),
    )
    .unwrap();
}

#[test]
fn env_create_activate_and_active_delete_have_distinct_reload_behavior() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let workdir = root.path().join("work");
    std::fs::create_dir_all(&workdir).unwrap();
    std::fs::write(
        workdir.join("opencoder.json"),
        serde_json::json!({"model": "openai/base-model"}).to_string(),
    )
    .unwrap();
    let _isolation = opencoder_core::scoped_config_home(home.clone());

    let (outcome, next) = execute(&workdir, EnvCommand::Create("alpha".into()));
    assert!(matches!(outcome, ModelOutcome::Idle));
    let Some(ModelMenu::Env(menu)) = next else {
        panic!("creation keeps the manager open")
    };
    assert_eq!(menu.entries.len(), 1);
    assert!(!menu.entries[0].active);
    assert_eq!(config_env_status().unwrap().active, None);
    write_env_config(
        &home,
        "alpha",
        serde_json::json!({"model": "openai/env-model", "context_limit": 12_000}),
    );

    let (outcome, next) = execute(&workdir, EnvCommand::Activate("alpha".into()));
    let ModelOutcome::ReloadEnv(config, label) = outcome else {
        panic!("activation must request a runtime reload")
    };
    assert!(next.is_none());
    assert_eq!(config.model, "openai/env-model");
    assert_eq!(config.context_limit(), 12_000);
    assert!(label.contains("alpha"));
    assert_eq!(
        config_env_status().unwrap().active.as_deref(),
        Some("alpha")
    );

    let selection = ModeSelection::new(
        Config::default(),
        ConfigMode::Act,
        ModeMenuKind::Model,
        &workdir,
        None,
    );
    assert_eq!(selection.save_target.active_env.as_deref(), Some("alpha"));
    assert_eq!(
        selection.save_target.path,
        home.join(".opencoder/envs/alpha/config.json")
            .display()
            .to_string()
    );

    let (outcome, next) = execute(&workdir, EnvCommand::Delete("alpha".into()));
    let ModelOutcome::ReloadEnv(config, _) = outcome else {
        panic!("deleting the active Env must reload base configuration")
    };
    assert!(next.is_none());
    assert_eq!(config.model, "openai/base-model");
    assert_eq!(config_env_status().unwrap().active, None);
    assert!(!home.join(".opencoder/envs/alpha").exists());
}

#[test]
fn save_target_views_follow_model_and_config_scope_routes() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let workdir = root.path().join("work");
    std::fs::create_dir_all(&workdir).unwrap();
    std::fs::write(workdir.join("opencoder.json"), "{}\n").unwrap();
    let _isolation = opencoder_core::scoped_config_home(home.clone());
    create_config_env("alpha").unwrap();
    activate_config_env("alpha").unwrap();

    let model = ModeSelection::new(
        Config::default(),
        ConfigMode::Act,
        ModeMenuKind::Model,
        &workdir,
        None,
    );
    assert_eq!(model.save_target.route, ConfigSaveRoute::ActiveEnv);
    assert_eq!(model.save_target.active_env.as_deref(), Some("alpha"));

    let shared = ModeSelection::new(
        Config::default(),
        ConfigMode::Shared,
        ModeMenuKind::Config,
        &workdir,
        None,
    );
    assert_eq!(shared.save_target.route, ConfigSaveRoute::Project);
    assert_eq!(shared.save_target.active_env.as_deref(), Some("alpha"));
    assert_eq!(shared.save_target.env_label(), "alpha");
    assert_eq!(
        shared.save_target.path,
        workdir.join("opencoder.json").display().to_string()
    );

    let act = ModeSelection::new(
        Config::default(),
        ConfigMode::Act,
        ModeMenuKind::Config,
        &workdir,
        None,
    );
    assert_eq!(act.save_target.route, ConfigSaveRoute::ActiveEnv);
    assert_eq!(act.save_target.active_env.as_deref(), Some("alpha"));
}

#[test]
fn invalid_env_preflight_keeps_previous_active_env_and_shows_error() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let workdir = root.path().join("work");
    std::fs::create_dir_all(&workdir).unwrap();
    std::fs::write(workdir.join("opencoder.json"), "{}\n").unwrap();
    let _isolation = opencoder_core::scoped_config_home(home.clone());

    let (_, next) = execute(&workdir, EnvCommand::Create("stable".into()));
    assert!(matches!(next, Some(ModelMenu::Env(_))));
    let (outcome, _) = execute(&workdir, EnvCommand::Activate("stable".into()));
    assert!(matches!(outcome, ModelOutcome::ReloadEnv(_, _)));
    let (_, next) = execute(&workdir, EnvCommand::Create("broken".into()));
    assert!(matches!(next, Some(ModelMenu::Env(_))));
    write_env_config(&home, "broken", serde_json::json!({"model": 42}));

    let (outcome, next) = execute(&workdir, EnvCommand::Activate("broken".into()));
    assert!(matches!(outcome, ModelOutcome::Idle));
    let Some(ModelMenu::Env(menu)) = next else {
        panic!("preflight failure keeps the manager open")
    };
    assert!(menu
        .error
        .as_deref()
        .is_some_and(|error| error.contains("broken")));
    assert_eq!(
        config_env_status().unwrap().active.as_deref(),
        Some("stable")
    );
}

#[test]
fn inactive_delete_refreshes_without_reloading_and_usage_errors_stay_visible() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let workdir = root.path().join("work");
    std::fs::create_dir_all(&workdir).unwrap();
    std::fs::write(workdir.join("opencoder.json"), "{}\n").unwrap();
    let _isolation = opencoder_core::scoped_config_home(home.clone());

    execute(&workdir, EnvCommand::Create("temporary".into()));
    let (outcome, next) = execute(&workdir, EnvCommand::Delete("temporary".into()));
    assert!(matches!(outcome, ModelOutcome::Idle));
    let Some(ModelMenu::Env(menu)) = next else {
        panic!("inactive deletion keeps the manager open")
    };
    assert!(menu.entries.is_empty());
    assert!(!home.join(".opencoder/envs/temporary").exists());

    let (outcome, next) = execute(
        &workdir,
        EnvCommand::Invalid("usage: /env create <name>".into()),
    );
    assert!(matches!(outcome, ModelOutcome::Idle));
    let Some(ModelMenu::Env(menu)) = next else {
        panic!("usage failure keeps the manager open")
    };
    assert_eq!(menu.error.as_deref(), Some("usage: /env create <name>"));
}
