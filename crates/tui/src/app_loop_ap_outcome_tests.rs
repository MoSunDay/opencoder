//! `/ap` confirm-dialog outcome tests (`handle_ap_outcome`).
//!
//! Split out of `app_loop_tests.rs` to keep that file under the 800-line cap.
//! Mirrors the `#[path]` convention used by `app_loop_session_only_tests.rs`.

#[allow(unused_imports)]
use super::super::*;
use super::*;

// ----- /ap confirm-dialog (Save / SaveSessionOnly / Cancel) tests -----
//
// The two-step `/ap` flow: Enter arms the "save as default?" prompt, then
// `y` persists globally (domain-routed `ap.json` + reload), `n` merges
// session-only (no disk write), Esc cancels with no effects.

use crate::chat::ChatBlock;
use opencoder_core::ApMode;

/// Marker texts pushed into the chat view (spans concatenated per line).
fn marker_texts(chat: &crate::chat::ChatView) -> Vec<String> {
    chat.blocks
        .iter()
        .filter_map(|b| match b {
            ChatBlock::Marker(lines) => Some(
                lines
                    .iter()
                    .flat_map(|l| l.spans.iter())
                    .map(|s| s.content.as_ref())
                    .collect::<String>(),
            ),
            _ => None,
        })
        .collect()
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn ap_session_only_merges_memory_and_skips_disk() {
    use crate::ap_menu::ApMenu;
    use crate::worker::UiCmd;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use opencoder_core::Config;

    let _guard = HOME_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();
    let _iso = opencoder_core::scoped_config_home(workdir.to_path_buf());

    let mut config = Config::default(); // autopilot off
    let mut ap_menu = Some(ApMenu::new(&config)); // cursor on "off"
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(64);
    let mut chat = crate::chat::ChatView::default();

    let down = KeyEvent::new(KeyCode::Down, KeyModifiers::empty());
    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::empty());
    let n_key = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::empty());

    // Step 1: Down onto "ap", Enter arms the prompt (Idle, menu stays open).
    handle_ap_outcome(&mut ap_menu, down, &mut config, &cmd_tx, &mut chat, workdir).await;
    let flow = handle_ap_outcome(
        &mut ap_menu,
        enter,
        &mut config,
        &cmd_tx,
        &mut chat,
        workdir,
    )
    .await;
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(ap_menu.is_some(), "menu stays open while prompting");

    // Step 2: 'n' applies the mode session-only.
    let flow = handle_ap_outcome(
        &mut ap_menu,
        n_key,
        &mut config,
        &cmd_tx,
        &mut chat,
        workdir,
    )
    .await;
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(ap_menu.is_none(), "modal closes after session-only switch");

    // In-memory config merged to the new mode.
    assert_eq!(config.autopilot.mode, ApMode::Ap);

    // ApModeSwitch dispatched (worker pins the override + persists the column).
    let cmd = cmd_rx.recv().await.expect("ApModeSwitch must be sent");
    assert!(matches!(cmd, UiCmd::ApModeSwitch(ApMode::Ap)));
    assert!(cmd_rx.try_recv().is_err(), "exactly one command");

    // CRITICAL: session-only = no disk write anywhere.
    assert!(
        !workdir.join("opencoder.json").exists(),
        "session-only switch must NOT persist opencoder.json"
    );
    assert!(
        !workdir.join(".opencoder").join("ap.json").exists(),
        "session-only switch must NOT persist ap.json"
    );

    // A "(session)" marker was pushed.
    let markers = marker_texts(&chat);
    assert!(
        markers.iter().any(|m| m.contains("(session)")),
        "expected a '(session)' marker, got: {markers:?}"
    );
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn ap_global_save_writes_config_and_notifies() {
    use crate::ap_menu::ApMenu;
    use crate::worker::UiCmd;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use opencoder_core::Config;

    let _guard = HOME_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();
    let _iso = opencoder_core::scoped_config_home(workdir.to_path_buf());

    let mut config = Config::default(); // autopilot off
    let mut ap_menu = Some(ApMenu::new(&config)); // cursor on "off"
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(64);
    let mut chat = crate::chat::ChatView::default();

    let down = KeyEvent::new(KeyCode::Down, KeyModifiers::empty());
    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::empty());
    let y_key = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::empty());

    // Down onto "ap", Enter arms the prompt, 'y' saves globally.
    handle_ap_outcome(&mut ap_menu, down, &mut config, &cmd_tx, &mut chat, workdir).await;
    handle_ap_outcome(
        &mut ap_menu,
        enter,
        &mut config,
        &cmd_tx,
        &mut chat,
        workdir,
    )
    .await;
    let flow = handle_ap_outcome(
        &mut ap_menu,
        y_key,
        &mut config,
        &cmd_tx,
        &mut chat,
        workdir,
    )
    .await;
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(ap_menu.is_none(), "modal closes after the global save");

    // The autopilot patch is domain-routed to `.opencoder/ap.json` (its top
    // level IS the AutoPilotConfig body) — `opencoder.json` is never created
    // by a domain-only patch.
    let ap_json = workdir.join(".opencoder").join("ap.json");
    let raw = std::fs::read_to_string(&ap_json)
        .unwrap_or_else(|e| panic!("ap.json must exist after a global save: {e}"));
    let parsed: serde_json::Value = serde_json::from_str(&raw).expect("ap.json must be valid JSON");
    assert_eq!(
        parsed["mode"], "ap",
        "ap.json persists the mode; got: {raw}"
    );

    // The reloaded config honors it.
    assert_eq!(config.autopilot.mode, ApMode::Ap);

    // ApModeSwitch dispatched (worker pins the session column too).
    let cmd = cmd_rx.recv().await.expect("ApModeSwitch must be sent");
    assert!(matches!(cmd, UiCmd::ApModeSwitch(ApMode::Ap)));

    // A "(global default)" marker was pushed.
    let markers = marker_texts(&chat);
    assert!(
        markers.iter().any(|m| m.contains("(global default)")),
        "expected a '(global default)' marker, got: {markers:?}"
    );
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn ap_esc_from_confirm_cancels_without_effects() {
    use crate::ap_menu::ApMenu;
    use crate::worker::UiCmd;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use opencoder_core::Config;

    let _guard = HOME_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();
    let _iso = opencoder_core::scoped_config_home(workdir.to_path_buf());

    let mut config = Config::default(); // autopilot off
    let mut ap_menu = Some(ApMenu::new(&config));
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(64);
    let mut chat = crate::chat::ChatView::default();

    let down = KeyEvent::new(KeyCode::Down, KeyModifiers::empty());
    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::empty());
    let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::empty());

    // Down onto "ap", Enter arms the prompt, Esc cancels everything.
    handle_ap_outcome(&mut ap_menu, down, &mut config, &cmd_tx, &mut chat, workdir).await;
    handle_ap_outcome(
        &mut ap_menu,
        enter,
        &mut config,
        &cmd_tx,
        &mut chat,
        workdir,
    )
    .await;
    let flow = handle_ap_outcome(&mut ap_menu, esc, &mut config, &cmd_tx, &mut chat, workdir).await;
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(ap_menu.is_none(), "modal closes on cancel");

    // No side effects: no disk write, no command, no mode change.
    assert!(!workdir.join("opencoder.json").exists());
    assert!(!workdir.join(".opencoder").join("ap.json").exists());
    assert!(
        cmd_rx.try_recv().is_err(),
        "cancel must not notify the worker"
    );
    assert_eq!(config.autopilot.mode, ApMode::Off, "config unchanged");
    assert!(marker_texts(&chat).is_empty(), "cancel posts no marker");
}
