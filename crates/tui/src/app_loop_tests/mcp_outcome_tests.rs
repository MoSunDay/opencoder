//! Tests for `handle_mcp_outcome` — the only MCP handler with persistence
//! side-effects (Config::save → reload → ReloadConfig). Three branches:
//! success, save-failure, reload-failure. Each pins its chat-marker text and
//! the ReloadConfig dispatch (or absence thereof).

use crate::app::app_loop::*;
use crate::chat::{ChatBlock, ChatView};
use crate::mcp_menu::{McpField, McpForm, McpList, McpMenu};
use crate::worker::UiCmd;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::Config;

/// Parse a JSON file from disk (missing file panics with the io error).
fn read_json(path: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
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

fn enter_key() -> KeyEvent {
    KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
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

    let flow = handle_mcp_outcome(
        &mut mcp_menu,
        enter_key(),
        &mut config,
        &cmd_tx,
        &mut chat,
        workdir,
    )
    .await;

    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(mcp_menu.is_none(), "modal should close after Save");

    let text = marker_text(&chat);
    assert!(
        text.contains("[/mcp] saved"),
        "expected saved marker, got: {text}"
    );

    let cmd = cmd_rx.recv().await.expect("ReloadConfig should be sent");
    assert!(matches!(cmd, UiCmd::ReloadConfig(_)));

    // The reloaded config should now carry the saved server.
    assert!(config.mcp_servers.contains_key("mysrv"));
}

/// Save-failure: a corrupt project `mcp.json` domain file (unparseable JSON)
/// causes `Config::save` to refuse the write (the corrupt bytes are left
/// untouched). An error marker is pushed and no `ReloadConfig` is dispatched.
#[tokio::test]
async fn handle_mcp_outcome_save_failure_pushes_error_marker() {
    let tmp = tempfile::tempdir().unwrap();
    let _iso = opencoder_core::scoped_config_home(tmp.path().to_path_buf());
    let workdir = tmp.path();

    // Corrupt the mcp domain file (project `<wd>/.opencoder/mcp.json`, which
    // is also the write target) so Config::save refuses to overwrite it.
    let corrupt_mcp = workdir.join(".opencoder").join("mcp.json");
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    std::fs::write(&corrupt_mcp, "{ broken json").unwrap();

    let mut mcp_menu = Some(form_ready_to_save());
    let mut config = Config::default();
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(64);
    let mut chat = ChatView::default();

    let flow = handle_mcp_outcome(
        &mut mcp_menu,
        enter_key(),
        &mut config,
        &cmd_tx,
        &mut chat,
        workdir,
    )
    .await;

    assert!(matches!(flow, LoopFlow::Proceed));

    let text = marker_text(&chat);
    assert!(
        text.contains("[/mcp] save failed"),
        "expected save-failed marker, got: {text}"
    );

    // No ReloadConfig on save failure.
    assert!(
        cmd_rx.try_recv().is_err(),
        "ReloadConfig must not be sent on save failure"
    );

    // The corrupt domain file must be left byte-for-byte untouched.
    assert_eq!(
        std::fs::read_to_string(&corrupt_mcp).unwrap(),
        "{ broken json",
        "a refused domain-file save must not modify the corrupt target"
    );
}

/// Reload-failure: the mcp patch saves fine into the global `mcp.json`
/// domain file (a domain-only patch never touches config.json), but the
/// reload hits a corrupt *global* config.json candidate (`.opencoder/
/// config.json` under the isolated home) and fails. An error marker is
/// pushed and no `ReloadConfig` is dispatched.
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

    let flow = handle_mcp_outcome(
        &mut mcp_menu,
        enter_key(),
        &mut config,
        &cmd_tx,
        &mut chat,
        &workdir,
    )
    .await;

    assert!(matches!(flow, LoopFlow::Proceed));

    let text = marker_text(&chat);
    assert!(
        text.contains("[/mcp] reload failed"),
        "expected reload-failed marker, got: {text}"
    );

    // No ReloadConfig on reload failure.
    assert!(
        cmd_rx.try_recv().is_err(),
        "ReloadConfig must not be sent on reload failure"
    );

    // The save itself succeeded: the server landed in the global domain file
    // (`<scoped-home>/.opencoder/mcp.json`, whose top level IS the server
    // map — no `mcp_servers` wrapper), not in config.json.
    let saved = read_json(&tmp.path().join(".opencoder").join("mcp.json"));
    assert_eq!(
        saved["mysrv"]["enabled"], false,
        "the mcp patch must persist to mcp.json even when the reload fails"
    );
    assert_eq!(
        std::fs::read_to_string(tmp.path().join(".opencoder").join("config.json")).unwrap(),
        "{ corrupt global",
        "the corrupt config.json candidate must stay untouched"
    );
}

/// Domain-file routing (the config.json hard-cut): Save / toggle / delete
/// from the `/mcp` menu all land in `mcp.json` — config.json is never
/// created or touched. With no pre-existing domain file the write target is
/// the global one under the scoped home; a null-delete leaves the key absent.
#[tokio::test]
async fn handle_mcp_outcome_save_toggle_delete_write_mcp_domain_file() {
    let tmp = tempfile::tempdir().unwrap();
    let _iso = opencoder_core::scoped_config_home(tmp.path().to_path_buf());
    // Distinct project dir so the project candidate (`project/.opencoder/
    // mcp.json`) and the global one (`<scoped-home>/.opencoder/mcp.json`)
    // are different paths — proving which one the write picked.
    let workdir = tmp.path().join("project");
    std::fs::create_dir_all(&workdir).unwrap();
    let global_mcp = tmp.path().join(".opencoder").join("mcp.json");

    let mut config = Config::default();
    let (cmd_tx, _cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(64);
    let mut chat = ChatView::default();

    // 1) Save from a filled form → creates the global mcp.json (the project
    //    file does not exist, so the global file is the write target).
    let mut mcp_menu = Some(form_ready_to_save());
    handle_mcp_outcome(
        &mut mcp_menu,
        enter_key(),
        &mut config,
        &cmd_tx,
        &mut chat,
        &workdir,
    )
    .await;

    let saved = read_json(&global_mcp);
    // The domain file's top level IS the server map — no `mcp_servers` wrapper.
    assert_eq!(saved["mysrv"]["enabled"], false);
    assert_eq!(saved["mysrv"]["inject_to"], serde_json::json!(["parent"]));
    assert!(
        saved["mysrv"].get("command").is_none(),
        "empty optional transport fields must be omitted"
    );
    assert_eq!(
        saved.as_object().map(|o| o.len()),
        Some(1),
        "exactly one server entry"
    );

    // 2) Toggle from the reloaded list → flips `enabled` in the same file,
    //    keeping the sibling fields from step 1.
    let mut mcp_menu = Some(McpMenu::List(McpList::new(&config)));
    handle_mcp_outcome(
        &mut mcp_menu,
        key(KeyCode::Right),
        &mut config,
        &cmd_tx,
        &mut chat,
        &workdir,
    )
    .await;

    let toggled = read_json(&global_mcp);
    assert_eq!(toggled["mysrv"]["enabled"], true);
    assert_eq!(
        toggled["mysrv"]["inject_to"],
        serde_json::json!(["parent"]),
        "a toggle patch must not drop sibling fields"
    );

    // 3) Delete: 'd' arms the confirmation, 'y' confirms (null merge-patch)
    //    → the key is gone from the domain file and the reloaded config.
    let mut mcp_menu = Some(McpMenu::List(McpList::new(&config)));
    handle_mcp_outcome(
        &mut mcp_menu,
        key(KeyCode::Char('d')),
        &mut config,
        &cmd_tx,
        &mut chat,
        &workdir,
    )
    .await;
    handle_mcp_outcome(
        &mut mcp_menu,
        key(KeyCode::Char('y')),
        &mut config,
        &cmd_tx,
        &mut chat,
        &workdir,
    )
    .await;
    assert!(
        mcp_menu.is_none(),
        "the list closes after a confirmed delete"
    );

    let deleted = read_json(&global_mcp);
    assert!(
        deleted.get("mysrv").is_none(),
        "a null-delete must leave the key absent from mcp.json"
    );
    assert!(
        config.mcp_servers.is_empty(),
        "the reloaded config must drop the deleted server"
    );

    // A domain-only patch must never create or touch any config.json — and
    // the project domain file was never created either (writes went global).
    assert!(!tmp.path().join(".opencoder").join("config.json").exists());
    assert!(!workdir.join("opencoder.json").exists());
    assert!(!workdir.join(".opencoder").join("mcp.json").exists());
}
