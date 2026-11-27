//! Tests for `handle_cli_outcome` — the `/cli` modal's persistence layer.
//! Mirrors `mcp_outcome_tests.rs`: every Save (form save / list toggle /
//! confirmed delete) must land in the `cli.json` domain file, and a
//! domain-only patch must never create or touch `config.json`.

use super::{auto_ack_commands, HOME_TEST_LOCK};
use crate::app::app_loop::*;
use crate::chat::{ChatBlock, ChatView};
use crate::cli_menu::{CliField, CliForm, CliList, CliMenu};
use crate::worker::UiCmd;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::Config;

/// Parse a JSON file from disk (missing file panics with the io error).
fn read_json(path: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

/// Collect all marker-block text into a flat `String` for substring asserts.
fn marker_text(chat: &ChatView) -> String {
    chat.blocks
        .iter()
        .filter_map(|b| match b {
            ChatBlock::Marker(lines) => Some(lines.as_slice()),
            _ => None,
        })
        .flat_map(|lines| lines.iter())
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect()
}

/// Build a `CliMenu::Form` with a valid entry name ready to Save on Enter.
fn form_ready_to_save() -> CliMenu {
    let mut form = CliForm::new_blank();
    form.name = "mycli".into();
    form.name_cursor = 5;
    form.field = CliField::Name;
    CliMenu::Form(form)
}

/// Domain-file routing for the `/cli` menu: Save / toggle / delete all land
/// in `cli.json` (global under the scoped home when no file pre-exists) and
/// `config.json` is never created. A null-delete leaves the key absent.
#[tokio::test]
async fn handle_cli_outcome_save_toggle_delete_write_cli_domain_file() {
    let tmp = tempfile::tempdir().unwrap();
    let _iso = opencoder_core::scoped_config_home(tmp.path().to_path_buf());
    // Distinct project dir so the project candidate (`project/.opencoder/
    // cli.json`) and the global one (`<scoped-home>/.opencoder/cli.json`)
    // are different paths — proving which one the write picked.
    let workdir = tmp.path().join("project");
    std::fs::create_dir_all(&workdir).unwrap();
    let global_cli = tmp.path().join(".opencoder").join("cli.json");

    let mut config = Config::default();
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(64);
    let mut cmd_rx = auto_ack_commands(cmd_rx);
    let mut chat = ChatView::default();

    // 1) Save from a filled form → creates the global cli.json.
    let mut cli_menu = Some(form_ready_to_save());
    handle_cli_outcome(
        &mut cli_menu,
        key(KeyCode::Enter),
        &mut config,
        &cmd_tx,
        &mut chat,
        &workdir,
    )
    .await;

    assert!(
        cli_menu.is_none(),
        "the form must close after a successful Save"
    );
    assert!(
        marker_text(&chat).contains("[/cli] saved"),
        "expected a saved marker, got: {}",
        marker_text(&chat)
    );
    assert!(
        matches!(cmd_rx.recv().await, Some(UiCmd::ReloadConfig { .. })),
        "a successful save must dispatch ReloadConfig"
    );

    let saved = read_json(&global_cli);
    // The domain file's top level IS the entry map — no `cli` wrapper.
    assert_eq!(saved["mycli"]["enabled"], false);
    assert_eq!(saved["mycli"]["inject_to"], serde_json::json!(["parent"]));
    assert_eq!(saved["mycli"]["content"], "");
    assert_eq!(
        saved.as_object().map(|o| o.len()),
        Some(1),
        "exactly one cli entry"
    );

    // 2) Toggle from the reloaded list → flips `enabled`, siblings kept.
    let mut cli_menu = Some(CliMenu::List(CliList::new(&config)));
    handle_cli_outcome(
        &mut cli_menu,
        key(KeyCode::Right),
        &mut config,
        &cmd_tx,
        &mut chat,
        &workdir,
    )
    .await;

    let toggled = read_json(&global_cli);
    assert_eq!(toggled["mycli"]["enabled"], true);
    assert_eq!(
        toggled["mycli"]["content"], "",
        "a toggle patch must not drop sibling fields"
    );

    // 3) Delete: 'd' arms the confirmation, 'y' confirms (null merge-patch)
    //    → the key is gone from the domain file and the reloaded config.
    let mut cli_menu = Some(CliMenu::List(CliList::new(&config)));
    handle_cli_outcome(
        &mut cli_menu,
        key(KeyCode::Char('d')),
        &mut config,
        &cmd_tx,
        &mut chat,
        &workdir,
    )
    .await;
    handle_cli_outcome(
        &mut cli_menu,
        key(KeyCode::Char('y')),
        &mut config,
        &cmd_tx,
        &mut chat,
        &workdir,
    )
    .await;
    assert!(
        cli_menu.is_none(),
        "the list closes after a confirmed delete"
    );

    let deleted = read_json(&global_cli);
    assert!(
        deleted.get("mycli").is_none(),
        "a null-delete must leave the key absent from cli.json"
    );
    assert!(
        config.cli.is_empty(),
        "the reloaded config must drop the deleted cli entry"
    );

    // A domain-only patch must never create any config.json — and the
    // project domain file was never created either (writes went global).
    assert!(!tmp.path().join(".opencoder").join("config.json").exists());
    assert!(!workdir.join("opencoder.json").exists());
    assert!(!workdir.join(".opencoder").join("cli.json").exists());
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn cli_save_uses_the_active_env_domain_file() {
    let _guard = HOME_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let _iso = opencoder_core::scoped_config_home(home.clone());
    opencoder_core::create_config_env("alpha").unwrap();
    opencoder_core::activate_config_env("alpha").unwrap();
    let workdir = tmp.path().join("project");
    std::fs::create_dir_all(&workdir).unwrap();

    let mut config = Config {
        model: "session-only/shared".into(),
        ..Default::default()
    };
    config.providers.insert(
        "session-only".into(),
        opencoder_core::ProviderConfig {
            base_url: "https://example.invalid/session".into(),
            api_key: Some("placeholder".into()),
            ..Default::default()
        },
    );
    config.modes.act = Some(opencoder_core::ModeConfig {
        model: Some("session-only/act".into()),
        ..Default::default()
    });
    config.skills.insert(
        "session-skill".into(),
        opencoder_core::config::SkillConfig { enabled: true },
    );
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(8);
    let mut cmd_rx = auto_ack_commands(cmd_rx);
    let mut chat = ChatView {
        shared_model: config.model.clone(),
        model: "session-only/act".into(),
        ..Default::default()
    };
    let mut menu = Some(form_ready_to_save());
    handle_cli_outcome(
        &mut menu,
        key(KeyCode::Enter),
        &mut config,
        &cmd_tx,
        &mut chat,
        &workdir,
    )
    .await;

    let env_cli = home.join(".opencoder/envs/alpha/cli.json");
    assert_eq!(read_json(&env_cli)["mycli"]["enabled"], false);
    assert!(config.cli.contains_key("mycli"));
    assert_eq!(config.model, "session-only/shared");
    assert!(config.providers.contains_key("session-only"));
    assert_eq!(
        config
            .modes
            .act
            .as_ref()
            .and_then(|mode| mode.model.as_deref()),
        Some("session-only/act")
    );
    assert!(config.skills.contains_key("session-skill"));
    assert_eq!(chat.shared_model, "session-only/shared");
    assert_eq!(chat.model, "session-only/act");
    let command = cmd_rx.recv().await.expect("CLI domain reload");
    match command {
        UiCmd::ReloadConfig {
            config: candidate,
            scope: crate::worker::ConfigReloadScope::Cli,
            ..
        } => {
            assert_eq!(candidate.model, "session-only/shared");
            assert!(candidate.providers.contains_key("session-only"));
            assert!(candidate.skills.contains_key("session-skill"));
            assert!(candidate.cli.contains_key("mycli"));
        }
        other => panic!("unexpected reload command: {other:?}"),
    }
    assert!(!home.join(".opencoder/cli.json").exists());
    assert!(!workdir.join(".opencoder/cli.json").exists());
    assert!(!workdir.join("opencoder.json").exists());
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn cli_reload_dispatch_failure_keeps_live_config_and_reports_disk_save() {
    let _guard = HOME_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let _iso = opencoder_core::scoped_config_home(tmp.path().to_path_buf());
    let workdir = tmp.path().join("project");
    std::fs::create_dir_all(&workdir).unwrap();
    let mut config = Config::default();
    let config_before = serde_json::to_value(&config).unwrap();
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(1);
    drop(cmd_rx);
    let mut chat = ChatView::default();
    let mut menu = Some(form_ready_to_save());

    handle_cli_outcome(
        &mut menu,
        key(KeyCode::Enter),
        &mut config,
        &cmd_tx,
        &mut chat,
        &workdir,
    )
    .await;

    assert_eq!(serde_json::to_value(&config).unwrap(), config_before);
    assert!(menu.is_none());
    assert!(tmp.path().join(".opencoder/cli.json").exists());
    let text = marker_text(&chat);
    assert!(text.contains("reload dispatch failed"), "{text}");
    assert!(text.contains("live config was not refreshed"), "{text}");
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn cli_worker_failure_keeps_outer_domain_and_reports_disk_save() {
    let _guard = HOME_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let _iso = opencoder_core::scoped_config_home(tmp.path().join("home"));
    let workdir = tmp.path().join("project");
    std::fs::create_dir_all(&workdir).unwrap();
    let mut config = Config::default();
    let before = serde_json::to_value(&config).unwrap();
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(4);
    let mut chat = ChatView::default();
    let mut menu = Some(form_ready_to_save());

    let (_, command) = tokio::join!(
        handle_cli_outcome(
            &mut menu,
            key(KeyCode::Enter),
            &mut config,
            &cmd_tx,
            &mut chat,
            &workdir,
        ),
        async {
            let command = cmd_rx.recv().await.expect("CLI reload command");
            match &command {
                UiCmd::ReloadConfig { ack: Some(ack), .. } => {
                    ack.send(Err("injected worker failure".into())).unwrap();
                }
                other => panic!("unexpected command: {other:?}"),
            }
            command
        }
    );

    assert!(matches!(
        command,
        UiCmd::ReloadConfig {
            scope: crate::worker::ConfigReloadScope::Cli,
            ..
        }
    ));
    assert_eq!(serde_json::to_value(&config).unwrap(), before);
    assert!(tmp.path().join("home/.opencoder/cli.json").exists());
    let text = marker_text(&chat);
    assert!(text.contains("runtime reload failed"), "{text}");
    assert!(text.contains("disk changes were saved"), "{text}");
    assert!(text.contains("live config was not refreshed"), "{text}");
}
