use std::sync::Arc;

use super::{auto_ack_commands, HOME_TEST_LOCK};
use crate::app::app_loop::*;
use crate::chat::ChatView;
use crate::command::EnvCommand;
use crate::model_menu::{
    ConfigField, EnvMenu, ModeField, ModeMenuKind, ModeSelection, ModelMenu, ProviderList,
};
use crate::worker::UiCmd;
use opencoder_core::{activate_config_env, create_config_env, Config, ConfigMode};
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_store::SessionModelOverrides;

fn write_base_config(workdir: &std::path::Path) {
    let value = serde_json::json!({
        "model": "openai/base-model",
        "provider": { "api_key": "test-placeholder" },
        "providers": {
            "deepseek": {
                "base_url": "https://example.invalid/v1",
                "api_key": "test-placeholder",
                "model": "chat-model"
            }
        }
    });
    std::fs::write(workdir.join("opencoder.json"), value.to_string()).unwrap();
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn scoped_model_save_routes_to_active_env_without_touching_project_file() {
    let _guard = HOME_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let workdir = root.path().join("work");
    std::fs::create_dir_all(&workdir).unwrap();
    write_base_config(&workdir);
    let project_before = std::fs::read_to_string(workdir.join("opencoder.json")).unwrap();
    let _isolation = opencoder_core::scoped_config_home(home.clone());
    create_config_env("alpha").unwrap();
    activate_config_env("alpha").unwrap();

    let loaded = Config::load(&workdir).unwrap();
    let selection = ModeSelection::new(
        loaded.clone(),
        ConfigMode::Act,
        ModeMenuKind::Model,
        &workdir,
        Some(SessionModelOverrides::default()),
    );
    let mut menu = Some(ModelMenu::List(ProviderList::for_mode(
        selection,
        ConfigMode::Act,
    )));
    let mut client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
    let mut config = loaded.clone();
    let mut threshold = config.compaction.context_threshold;
    let mut context_limit = config.context_limit();
    let mut frame_ms = config.tui_frame_ms();
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(frame_ms));
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(4);
    let mut cmd_rx = auto_ack_commands(cmd_rx);
    let mut chat = ChatView {
        agent: "act".into(),
        shared_model: loaded.model.clone(),
        model: loaded.effective_for_mode(ConfigMode::Act).model,
        model_overrides: Some(SessionModelOverrides::default()),
        ..Default::default()
    };
    let enter = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    );
    handle_model_outcome(
        &mut menu,
        enter,
        &mut client,
        &mut config,
        &mut threshold,
        &mut context_limit,
        &mut frame_ms,
        &mut ticker,
        &cmd_tx,
        &Default::default(),
        &mut chat,
        &workdir,
        None,
        false,
    )
    .await;
    let save = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('y'),
        crossterm::event::KeyModifiers::NONE,
    );
    handle_model_outcome(
        &mut menu,
        save,
        &mut client,
        &mut config,
        &mut threshold,
        &mut context_limit,
        &mut frame_ms,
        &mut ticker,
        &cmd_tx,
        &Default::default(),
        &mut chat,
        &workdir,
        None,
        false,
    )
    .await;

    assert_eq!(
        std::fs::read_to_string(workdir.join("opencoder.json")).unwrap(),
        project_before
    );
    let env_json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.join(".opencoder/envs/alpha/config.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(env_json["modes"]["act"]["model"], "deepseek/chat-model");
    assert_eq!(config.model, "openai/base-model");
    assert_eq!(chat.shared_model, "openai/base-model");
    assert_eq!(
        chat.model, "deepseek/chat-model",
        "act config: {:?}; overrides: {:?}",
        config.modes.act, chat.model_overrides
    );
    assert!(matches!(
        cmd_rx.recv().await,
        Some(UiCmd::ReloadConfig { config: reloaded, .. })
            if reloaded.modes.act.as_ref().and_then(|mode| mode.model.as_deref())
                == Some("deepseek/chat-model")
    ));
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn typed_env_activation_drains_through_runtime_and_refreshes_effective_view() {
    let _guard = HOME_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let workdir = root.path().join("work");
    std::fs::create_dir_all(&workdir).unwrap();
    write_base_config(&workdir);
    let _isolation = opencoder_core::scoped_config_home(home.clone());
    create_config_env("alpha").unwrap();
    std::fs::write(
        home.join(".opencoder/envs/alpha/config.json"),
        serde_json::json!({
            "model": "openai/env-shared",
            "context_limit": 12_000,
            "compaction": {"context_threshold": 9_000},
            "fps": 20,
            "providers": {"openai": {
                "base_url": "https://example.invalid/v1",
                "api_key": "test-placeholder",
                "model": "env-shared"
            }},
            "modes": {"act": {"model": "openai/env-act", "context_limit": 11_000}}
        })
        .to_string(),
    )
    .unwrap();

    let mut config = Config::load(&workdir).unwrap();
    let mut chat = ChatView {
        agent: "act".into(),
        shared_model: "openai/shared-temporary".into(),
        model: "openai/temporary".into(),
        model_overrides: Some(SessionModelOverrides {
            shared: Some("openai/shared-temporary".into()),
            act: Some("openai/temporary".into()),
            plan: Some("openai/plan-temporary".into()),
        }),
        ..Default::default()
    };
    let mut menu = Some(ModelMenu::Env(EnvMenu::with_command(
        &workdir,
        EnvCommand::Activate("alpha".into()),
    )));
    let mut client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
    let mut threshold = 0;
    let mut context_limit = 0;
    let mut frame_ms = config.tui_frame_ms();
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(frame_ms));
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel(4);
    let model_switches = Default::default();

    let (flow, command) = tokio::join!(
        drain_pending_env(
            &mut menu,
            &mut client,
            &mut config,
            &mut threshold,
            &mut context_limit,
            &mut frame_ms,
            &mut ticker,
            &cmd_tx,
            &model_switches,
            &mut chat,
            &workdir,
            None,
            false,
        ),
        async {
            let command = cmd_rx.recv().await.expect("ReloadEnv command");
            match &command {
                UiCmd::ReloadEnv { ack, .. } => ack.send(Ok(())).unwrap(),
                other => panic!("unexpected command: {other:?}"),
            }
            command
        }
    );

    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(menu.is_none());
    assert_eq!(config.model, "openai/env-shared");
    assert_eq!(chat.shared_model, "openai/env-shared");
    assert_eq!(chat.model, "openai/env-act");
    assert_eq!(chat.model_overrides, Some(SessionModelOverrides::default()));
    assert_eq!(context_limit, 11_000);
    assert_eq!(threshold, 9_000);
    assert_eq!(frame_ms, 50);
    assert!(matches!(
        command,
        UiCmd::ReloadEnv { ref config, .. } if config.model == "openai/env-shared"
    ));
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn act_config_uses_env_while_shared_config_stays_on_project() {
    let _guard = HOME_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let workdir = root.path().join("work");
    std::fs::create_dir_all(&workdir).unwrap();
    write_base_config(&workdir);
    let project_before = std::fs::read_to_string(workdir.join("opencoder.json")).unwrap();
    let _isolation = opencoder_core::scoped_config_home(home.clone());
    create_config_env("alpha").unwrap();
    activate_config_env("alpha").unwrap();

    let mut config = Config::load(&workdir).unwrap();
    let mut client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
    let mut threshold = config.compaction.context_threshold;
    let mut context_limit = config.context_limit();
    let mut frame_ms = config.tui_frame_ms();
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(frame_ms));
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(4);
    let _cmd_rx = auto_ack_commands(cmd_rx);
    let mut chat = ChatView::default();
    let enter = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    );
    let mut menu = Some(ModelMenu::ModeSelect(Box::new(ModeSelection::new(
        config.clone(),
        ConfigMode::Act,
        ModeMenuKind::Config,
        &workdir,
        Some(SessionModelOverrides::default()),
    ))));
    handle_model_outcome(
        &mut menu,
        enter,
        &mut client,
        &mut config,
        &mut threshold,
        &mut context_limit,
        &mut frame_ms,
        &mut ticker,
        &cmd_tx,
        &Default::default(),
        &mut chat,
        &workdir,
        None,
        false,
    )
    .await;
    let Some(ModelMenu::ModeConfig(form)) = menu.as_mut() else {
        panic!("Act config form must open")
    };
    form.max_tokens_input = "8192".into();
    form.focus = ModeField::Save;
    handle_model_outcome(
        &mut menu,
        enter,
        &mut client,
        &mut config,
        &mut threshold,
        &mut context_limit,
        &mut frame_ms,
        &mut ticker,
        &cmd_tx,
        &Default::default(),
        &mut chat,
        &workdir,
        None,
        false,
    )
    .await;

    let env_path = home.join(".opencoder/envs/alpha/config.json");
    let env_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&env_path).unwrap()).unwrap();
    assert_eq!(env_json["modes"]["act"]["max_tokens"], 8192);
    assert_eq!(
        std::fs::read_to_string(workdir.join("opencoder.json")).unwrap(),
        project_before
    );

    let mut menu = Some(ModelMenu::ModeSelect(Box::new(ModeSelection::new(
        config.clone(),
        ConfigMode::Shared,
        ModeMenuKind::Config,
        &workdir,
        Some(SessionModelOverrides::default()),
    ))));
    handle_model_outcome(
        &mut menu,
        enter,
        &mut client,
        &mut config,
        &mut threshold,
        &mut context_limit,
        &mut frame_ms,
        &mut ticker,
        &cmd_tx,
        &Default::default(),
        &mut chat,
        &workdir,
        None,
        false,
    )
    .await;
    let Some(ModelMenu::Config(form)) = menu.as_mut() else {
        panic!("Shared config form must open")
    };
    form.fps_input = "17".into();
    form.focus = ConfigField::Save;
    handle_model_outcome(
        &mut menu,
        enter,
        &mut client,
        &mut config,
        &mut threshold,
        &mut context_limit,
        &mut frame_ms,
        &mut ticker,
        &cmd_tx,
        &Default::default(),
        &mut chat,
        &workdir,
        None,
        false,
    )
    .await;

    let project_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(workdir.join("opencoder.json")).unwrap())
            .unwrap();
    assert_eq!(project_json["fps"], 17);
    assert!(project_json.get("modes").is_none());
    let env_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&env_path).unwrap()).unwrap();
    assert!(env_json.get("fps").is_none());
    assert!(env_json.get("enable_tmux_session").is_none());
    assert_eq!(env_json["modes"]["act"]["max_tokens"], 8192);
}
