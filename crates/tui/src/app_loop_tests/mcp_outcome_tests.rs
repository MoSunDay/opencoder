//! Tests for `handle_mcp_outcome` — the only MCP handler with persistence
//! side-effects (Config::save → reload → ReloadConfig). Three branches:
//! success, save-failure, reload-failure. Each pins its chat-marker text and
//! the ReloadConfig dispatch (or absence thereof).

use crate::app::app_loop::*;
use crate::chat::{ChatBlock, ChatView};
use crate::mcp_menu::{McpField, McpForm, McpMenu};
use crate::worker::UiCmd;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::Config;

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

fn enter_key() -> KeyEvent {
    KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())
}

/// Build a `McpMenu::Form` with a valid server name ready to Save on Enter.
fn form_ready_to_save() -> McpMenu {
    let mut form = McpForm::new_blank();
    form.name = "mysrv".into();
    form.name_cursor = 5;
    form.field = McpField::Name;
    McpMenu::Form(form)
}

/// Success: Save → reload → ReloadConfig dispatched + "[/mcp] saved" marker.
/// Config discovery is isolated to a tempdir so no host config interferes.
#[tokio::test]
async fn handle_mcp_outcome_success_saves_and_reloads() {
    let tmp = tempfile::tempdir().unwrap();
    let _iso = opencoder_core::scoped_config_home(tmp.path().to_path_buf());
    let workdir = tmp.path();

    let mut mcp_menu = Some(form_ready_to_save());
    let mut config = Config::default();
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(64);
    let mut chat = ChatView::default();

    let flow = handle_mcp_outcome(&mut mcp_menu, enter_key(), &mut config, &cmd_tx, &mut chat, workdir).await;

    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(mcp_menu.is_none(), "modal should close after Save");

    let text = marker_text(&chat);
    assert!(text.contains("[/mcp] saved"), "expected saved marker, got: {text}");

    let cmd = cmd_rx.recv().await.expect("ReloadConfig should be sent");
    assert!(matches!(cmd, UiCmd::ReloadConfig(_)));

    // The reloaded config should now carry the saved server.
    assert!(config.mcp_servers.contains_key("mysrv"));
}

/// Save-failure: a corrupt project-local `opencoder.json` (unparseable JSON)
/// causes `Config::save` to reject it. An error marker is pushed and no
/// `ReloadConfig` is dispatched.
#[tokio::test]
async fn handle_mcp_outcome_save_failure_pushes_error_marker() {
    let tmp = tempfile::tempdir().unwrap();
    let _iso = opencoder_core::scoped_config_home(tmp.path().to_path_buf());
    let workdir = tmp.path();

    // Corrupt the save target so Config::save refuses to overwrite it.
    std::fs::write(workdir.join("opencoder.json"), "{ broken json").unwrap();

    let mut mcp_menu = Some(form_ready_to_save());
    let mut config = Config::default();
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(64);
    let mut chat = ChatView::default();

    let flow = handle_mcp_outcome(&mut mcp_menu, enter_key(), &mut config, &cmd_tx, &mut chat, workdir).await;

    assert!(matches!(flow, LoopFlow::Proceed));

    let text = marker_text(&chat);
    assert!(text.contains("[/mcp] save failed"), "expected save-failed marker, got: {text}");

    // No ReloadConfig on save failure.
    assert!(cmd_rx.try_recv().is_err(), "ReloadConfig must not be sent on save failure");
}

/// Reload-failure: save succeeds (writes a fresh `opencoder.json`) but a
/// corrupt *global* candidate (`.opencoder/config.json` under the isolated
/// home) makes `Config::load` fail during reload. An error marker is pushed
/// and no `ReloadConfig` is dispatched.
#[tokio::test]
async fn handle_mcp_outcome_reload_failure_pushes_error_marker() {
    let tmp = tempfile::tempdir().unwrap();
    let _iso = opencoder_core::scoped_config_home(tmp.path().to_path_buf());

    // Use a subdirectory as the working dir so global candidates under the
    // isolated home are distinct from project-local files.
    let workdir = tmp.path().join("project");
    std::fs::create_dir_all(&workdir).unwrap();

    // Pre-write a corrupt global candidate that Config::load will encounter.
    std::fs::create_dir_all(tmp.path().join(".opencoder")).unwrap();
    std::fs::write(
        tmp.path().join(".opencoder").join("config.json"),
        "{ corrupt global",
    )
    .unwrap();

    let mut mcp_menu = Some(form_ready_to_save());
    let mut config = Config::default();
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(64);
    let mut chat = ChatView::default();

    let flow = handle_mcp_outcome(&mut mcp_menu, enter_key(), &mut config, &cmd_tx, &mut chat, &workdir).await;

    assert!(matches!(flow, LoopFlow::Proceed));

    let text = marker_text(&chat);
    assert!(text.contains("[/mcp] reload failed"), "expected reload-failed marker, got: {text}");

    // No ReloadConfig on reload failure.
    assert!(cmd_rx.try_recv().is_err(), "ReloadConfig must not be sent on reload failure");
}
