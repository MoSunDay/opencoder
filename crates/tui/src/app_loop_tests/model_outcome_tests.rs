//! Correlated runtime-commit tests for persisted model/config outcomes.

use std::sync::Arc;

use super::{auto_ack_commands, HOME_TEST_LOCK};
use crate::app::app_loop::*;
use crate::chat::{ChatBlock, ChatView};
use crate::model_menu::{
    ConfigField, ConfigForm, ModeConfigForm, ModeField, ModeMenuKind, ModeSelection, ModelMenu,
    ProviderList,
};
use crate::worker::{ConfigReloadScope, UiCmd};
use opencoder_core::{activate_config_env, create_config_env, Config, ConfigMode};
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_store::SessionModelOverrides;

fn enter() -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::empty(),
    )
}

fn marker_text(chat: &ChatView) -> String {
    chat.blocks
        .iter()
        .filter_map(|block| match block {
            ChatBlock::Marker(lines) => Some(lines.as_slice()),
            _ => None,
        })
        .flat_map(|lines| lines.iter())
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect()
}

enum WorkerReply {
    Ack(Result<(), String>),
    DropCommand,
    ClosedChannel,
}

struct SaveSnapshot {
    before: Config,
    outer: Config,
    disk_model: String,
    chat: ChatView,
    client_changed: bool,
    command: Option<UiCmd>,
}

#[allow(clippy::await_holding_lock)]
async fn run_shared_save(reply: WorkerReply, shared_override: Option<&str>) -> SaveSnapshot {
    let _guard = HOME_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let _isolation = opencoder_core::scoped_config_home(tmp.path().join("home"));
    let workdir = tmp.path().join("project");
    std::fs::create_dir_all(&workdir).unwrap();
    let before = Config {
        model: "openai/before".into(),
        provider: opencoder_core::ProviderConfig {
            api_key: Some("test-placeholder".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    std::fs::write(
        workdir.join("opencoder.json"),
        serde_json::to_string(&before).unwrap(),
    )
    .unwrap();

    let mut list = ProviderList::new(&before);
    list.confirm_save_default = Some(serde_json::json!({"model": "openai/after"}));
    let mut menu = Some(ModelMenu::List(list));
    let mut outer = before.clone();
    let mut client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
    let client_before = Arc::as_ptr(&client);
    let mut threshold = before.compaction.context_threshold;
    let mut context_limit = before.context_limit();
    let mut frame_ms = before.tui_frame_ms();
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(frame_ms));
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(4);
    let model_switches = Default::default();
    let mut chat = ChatView {
        shared_model: shared_override.unwrap_or(&before.model).to_string(),
        model: before.model.clone(),
        model_overrides: shared_override.map(|model| SessionModelOverrides {
            shared: Some(model.to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };

    let command = if matches!(reply, WorkerReply::ClosedChannel) {
        drop(cmd_rx);
        handle_model_outcome(
            &mut menu,
            enter(),
            &mut client,
            &mut outer,
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
        None
    } else {
        let (_, command) = tokio::join!(
            handle_model_outcome(
                &mut menu,
                enter(),
                &mut client,
                &mut outer,
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
                let command = cmd_rx.recv().await.expect("full reload command");
                match (&command, reply) {
                    (UiCmd::ReloadConfig { ack: Some(ack), .. }, WorkerReply::Ack(result)) => {
                        ack.send(result).unwrap();
                        Some(command)
                    }
                    (UiCmd::ReloadConfig { .. }, WorkerReply::DropCommand) => {
                        drop(command);
                        None
                    }
                    (_, WorkerReply::ClosedChannel) => unreachable!(),
                    (other, _) => panic!("unexpected worker command: {other:?}"),
                }
            }
        );
        command
    };

    SaveSnapshot {
        disk_model: Config::load(&workdir).unwrap().model,
        before,
        outer,
        chat,
        client_changed: !std::ptr::addr_eq(client_before, Arc::as_ptr(&client)),
        command,
    }
}

#[tokio::test]
async fn shared_store_failure_keeps_outer_unchanged_and_reports_disk_save() {
    let saved = run_shared_save(
        WorkerReply::Ack(Err(
            "config reload failed to update sessions.model: injected failure".into(),
        )),
        None,
    )
    .await;

    assert_eq!(saved.disk_model, "openai/after");
    assert_eq!(saved.outer.model, saved.before.model);
    assert_eq!(saved.chat.shared_model, saved.before.model);
    assert_eq!(saved.chat.model, saved.before.model);
    assert!(!saved.client_changed);
    let text = marker_text(&saved.chat);
    assert!(text.contains("runtime reload failed"), "{text}");
    assert!(text.contains("disk changes were saved"), "{text}");
    assert!(text.contains("live config was not refreshed"), "{text}");
}

#[tokio::test]
async fn shared_send_failure_keeps_outer_unchanged_and_reports_disk_save() {
    let saved = run_shared_save(WorkerReply::ClosedChannel, None).await;

    assert_eq!(saved.disk_model, "openai/after");
    assert_eq!(saved.outer.model, saved.before.model);
    assert_eq!(saved.chat.shared_model, saved.before.model);
    assert!(!saved.client_changed);
    assert!(saved.command.is_none());
    let text = marker_text(&saved.chat);
    assert!(text.contains("reload dispatch failed"), "{text}");
    assert!(text.contains("live config was not refreshed"), "{text}");
}

#[tokio::test]
async fn shared_success_ack_commits_outer_config_chat_and_client() {
    let saved = run_shared_save(WorkerReply::Ack(Ok(())), None).await;

    assert_eq!(saved.disk_model, "openai/after");
    assert_eq!(saved.outer.model, "openai/after");
    assert_eq!(saved.chat.shared_model, "openai/after");
    assert_eq!(saved.chat.model, "openai/after");
    assert!(saved.client_changed);
    assert!(matches!(
        saved.command,
        Some(UiCmd::ReloadConfig {
            scope: ConfigReloadScope::Full,
            ..
        })
    ));
    let text = marker_text(&saved.chat);
    assert!(text.contains("[/config] saved"), "{text}");
    assert!(!text.contains("runtime reload failed"), "{text}");
}

#[tokio::test]
async fn shared_full_save_preserves_existing_session_selection() {
    let saved = run_shared_save(WorkerReply::Ack(Ok(())), Some("openai/session-shared")).await;

    assert_eq!(saved.disk_model, "openai/after");
    assert_eq!(saved.outer.model, "openai/after");
    assert_eq!(saved.chat.shared_model, "openai/session-shared");
    assert_eq!(saved.chat.model, "openai/session-shared");
    assert_eq!(
        saved
            .chat
            .model_overrides
            .and_then(|items| items.shared)
            .as_deref(),
        Some("openai/session-shared")
    );
    assert!(saved.client_changed);
}

#[tokio::test]
async fn shared_missing_ack_keeps_outer_unchanged_and_marks_worker_unconfirmed() {
    let saved = run_shared_save(WorkerReply::DropCommand, None).await;

    assert_eq!(saved.outer.model, saved.before.model);
    assert_eq!(saved.chat.shared_model, saved.before.model);
    assert!(!saved.client_changed);
    let text = marker_text(&saved.chat);
    assert!(text.contains("without confirmation"), "{text}");
    assert!(text.contains("worker state is unconfirmed"), "{text}");
}

#[allow(clippy::await_holding_lock)]
async fn run_client_build_failure(disk: serde_json::Value, expected_error: &str) {
    let _guard = HOME_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let _isolation = opencoder_core::scoped_config_home(tmp.path().join("home"));
    let workdir = tmp.path();
    std::fs::write(workdir.join("opencoder.json"), disk.to_string()).unwrap();

    let before = Config::default();
    let mut form = ConfigForm::new(&before);
    form.threshold_input = "80000".into();
    form.focus = ConfigField::Save;
    let mut menu = Some(ModelMenu::Config(form));
    let mut outer = before.clone();
    let mut client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
    let client_before = Arc::as_ptr(&client);
    let mut threshold = 0;
    let mut context_limit = 0;
    let mut frame_ms = 25;
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(frame_ms));
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(4);
    let mut chat = ChatView::default();

    handle_model_outcome(
        &mut menu,
        enter(),
        &mut client,
        &mut outer,
        &mut threshold,
        &mut context_limit,
        &mut frame_ms,
        &mut ticker,
        &cmd_tx,
        &Default::default(),
        &mut chat,
        workdir,
        None,
        false,
    )
    .await;

    assert_eq!(
        serde_json::to_value(&outer).unwrap(),
        serde_json::to_value(&before).unwrap()
    );
    assert_eq!(client_before, Arc::as_ptr(&client));
    assert!(
        cmd_rx.try_recv().is_err(),
        "invalid client must not reach worker"
    );
    let text = marker_text(&chat);
    assert!(text.contains(expected_error), "{text}");
    assert!(text.contains("disk changes were saved"), "{text}");
    assert!(text.contains("live config was not refreshed"), "{text}");
}

#[tokio::test]
async fn client_build_failure_does_not_dispatch_or_commit_outer_state() {
    run_client_build_failure(
        serde_json::json!({
            "model": "openai/bad-proxy-model",
            "provider": { "api_key": "test-placeholder" },
            "network": { "proxy": "://nope" }
        }),
        "client build failed",
    )
    .await;
}

#[tokio::test]
async fn endpoint_resolve_failure_does_not_dispatch_or_commit_outer_state() {
    run_client_build_failure(
        serde_json::json!({
            "model": "openai/no-key-model",
            "provider": { "api_key": null }
        }),
        "endpoint resolve failed",
    )
    .await;
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn shared_mode_config_save_uses_auto_with_active_env() {
    let _guard = HOME_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let workdir = tmp.path().join("project");
    std::fs::create_dir_all(&workdir).unwrap();
    let before = Config {
        model: "openai/before".into(),
        provider: opencoder_core::ProviderConfig {
            api_key: Some("test-placeholder".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    std::fs::write(
        workdir.join("opencoder.json"),
        serde_json::to_string(&before).unwrap(),
    )
    .unwrap();
    let _isolation = opencoder_core::scoped_config_home(home.clone());
    create_config_env("alpha").unwrap();
    activate_config_env("alpha").unwrap();

    let mut outer = Config::load(&workdir).unwrap();
    let selection = ModeSelection::new(
        outer.clone(),
        ConfigMode::Shared,
        ModeMenuKind::Config,
        &workdir,
        Some(SessionModelOverrides::default()),
    );
    let mut form = ModeConfigForm::new(selection, ConfigMode::Shared);
    form.max_tokens_input = "8192".into();
    form.focus = ModeField::Save;
    let mut menu = Some(ModelMenu::ModeConfig(form));
    let mut client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
    let mut threshold = outer.compaction.context_threshold;
    let mut context_limit = outer.context_limit();
    let mut frame_ms = outer.tui_frame_ms();
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(frame_ms));
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(4);
    let _cmd_rx = auto_ack_commands(cmd_rx);
    let mut chat = ChatView::default();

    handle_model_outcome(
        &mut menu,
        enter(),
        &mut client,
        &mut outer,
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

    let project: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(workdir.join("opencoder.json")).unwrap())
            .unwrap();
    let env: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.join(".opencoder/envs/alpha/config.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(project["modes"]["shared"]["max_tokens"], 8192);
    assert!(env.get("modes").is_none());
}
