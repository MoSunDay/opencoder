//! Tests for `handle_cli_outcome` — the `/cli` modal's persistence layer.
//! Mirrors `mcp_outcome_tests.rs`: every Save (form save / list toggle /
//! confirmed delete) must land in the `cli.json` domain file, and a
//! domain-only patch must never create or touch `config.json`.

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
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(64);
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
        matches!(cmd_rx.recv().await, Some(UiCmd::ReloadConfig(_))),
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
        toggled["mycli"]["content"],
        "",
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
    assert!(cli_menu.is_none(), "the list closes after a confirmed delete");

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
