//! Session-only model-switch outcome tests.
//!
//! Split out of `app_loop_tests.rs` to keep that file under the 800-line cap.
//! Mirrors the `#[path]` convention used by `app_loop_bugfix_tests.rs`.

use super::super::*;
use super::*;

// ----- SaveSessionOnly (session-only model switch) tests -----
//
// The `/model` "save as default? (y/N)" flow: Enter arms the prompt, then
// n/Enter/Esc triggers `SaveSessionOnly` which hot-swaps the model in memory
// and dispatches `ReloadConfig` WITHOUT writing opencoder.json.

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn handle_model_outcome_session_only_skips_disk_write() {
    use crate::chat::ChatBlock;
    use crate::model_menu::{ModelMenu, ProviderList};
    use crate::worker::UiCmd;
    use opencoder_core::{Config, ProviderConfig};
    use opencoder_llm::MockChatClient;

    let _guard = HOME_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();

    // Config with a custom provider so the list has an entry to switch to.
    let mut config = Config {
        model: "openai/gpt-4o-mini".to_string(),
        provider: ProviderConfig {
            api_key: Some("k".to_string()),
            ..Default::default()
        },
        ..Config::default()
    };
    config.providers.insert(
        "deepseek".to_string(),
        ProviderConfig {
            base_url: "https://api.deepseek.com/v1".to_string(),
            api_key: Some("dk-secret".to_string()),
            model: Some("deepseek-chat".to_string()),
            ..Default::default()
        },
    );

    let mut model_menu = Some(ModelMenu::List(ProviderList::new(&config)));
    let mut client: std::sync::Arc<dyn opencoder_llm::ChatStream> =
        std::sync::Arc::new(MockChatClient::new());
    let mut model_label = config.model.clone();
    let mut compaction_threshold = config.compaction.context_threshold;
    let mut context_limit = config.context_limit();
    let mut frame_ms = 25u64;
    let mut frame_ticker = tokio::time::interval(std::time::Duration::from_millis(frame_ms));
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(64);
    let mut chat = crate::chat::ChatView::default();

    let enter = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::empty(),
    );
    let n_key = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('n'),
        crossterm::event::KeyModifiers::empty(),
    );

    // Step 1: Enter arms the "save as default?" prompt (returns Idle).
    let _ = handle_model_outcome(
        &mut model_menu,
        enter,
        &mut client,
        &mut config,
        &mut model_label,
        &mut compaction_threshold,
        &mut context_limit,
        &mut frame_ms,
        &mut frame_ticker,
        &cmd_tx,
        &mut chat,
        workdir,
    )
    .await;
    assert!(model_menu.is_some(), "menu stays open while prompting");

    // Step 2: 'n' triggers the session-only switch.
    let flow = handle_model_outcome(
        &mut model_menu,
        n_key,
        &mut client,
        &mut config,
        &mut model_label,
        &mut compaction_threshold,
        &mut context_limit,
        &mut frame_ms,
        &mut frame_ticker,
        &cmd_tx,
        &mut chat,
        workdir,
    )
    .await;
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(
        model_menu.is_none(),
        "modal closes after session-only switch"
    );

    // In-memory config updated to the new model.
    assert_eq!(config.model, "deepseek/deepseek-chat");
    assert_eq!(model_label, "deepseek/deepseek-chat");

    // ReloadConfig dispatched (worker persists to session store row).
    let cmd = cmd_rx.recv().await.expect("ReloadConfig must be sent");
    assert!(matches!(cmd, UiCmd::ReloadConfig(_)));

    // CRITICAL: opencoder.json must NOT exist (session-only = no disk write).
    assert!(
        !workdir.join("opencoder.json").exists(),
        "session-only switch must NOT persist opencoder.json"
    );

    // A "session only" marker was pushed.
    let markers: Vec<String> = chat
        .blocks
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
        .collect();
    assert!(
        markers.iter().any(|m| m.contains("session only")),
        "expected a 'session only' marker, got: {markers:?}"
    );
}
