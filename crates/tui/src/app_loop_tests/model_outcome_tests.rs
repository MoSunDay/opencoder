//! Model-outcome Err-branch tests extracted from the unified app_loop_tests
//! module to keep each file under the 800-line cap.

use super::{EnvGuard, HOME_TEST_LOCK};
use crate::app::app_loop::*;

// ----- handle_model_outcome Err-branch tests -----
//
// `handle_model_outcome` walks the save→reload→resolve_endpoint→ChatClient::new
// chain; the last two steps can fail. Each failure path must push a red error
// marker into `chat`, then still send `UiCmd::ReloadConfig` and a green "saved"
// marker (the reload/saved markers are pushed unconditionally after the inner
// match — see `app_loop.rs`). These two tests pin the error-marker text and
// the ReloadConfig dispatch for each Err branch.

/// `ChatClient::new` rejects an invalid proxy URL → the "client build failed"
/// red marker is pushed. The project-local `opencoder.json` pre-supplies a valid
/// api_key (so `resolve_endpoint` succeeds) plus a malformed proxy string; the
/// form's JSON merge-patch preserves the proxy because it isn't part of the
/// patch. A mutex guards against concurrent HOME-dependent tests.
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn handle_model_outcome_client_build_failure_pushes_red_marker() {
    use crate::chat::ChatBlock;
    use crate::model_menu::{ConfigField, ConfigForm, ModelMenu};
    use opencoder_core::Config;
    use opencoder_llm::MockChatClient;

    let _guard = HOME_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();

    // Pre-write a config with api_key present but a bad proxy URL.
    // `Config::save` merges the form's patch on top, preserving
    // model/provider/proxy.
    let config_json = serde_json::json!({
        "model": "openai/bad-proxy-model",
        "provider": { "api_key": "k" },
        "network": { "proxy": "://nope" }
    });
    std::fs::write(workdir.join("opencoder.json"), config_json.to_string()).unwrap();

    // Build a ConfigForm focused on the Save button.
    let base_cfg = Config::default();
    let mut form = ConfigForm::new(&base_cfg);
    form.threshold_input = "80000".into(); // ensure validation passes (>= 1000)
    form.focus = ConfigField::Save;
    let mut model_menu = Some(ModelMenu::Config(form));

    // Set up the rest of `handle_model_outcome`'s parameters.
    let mut client: std::sync::Arc<dyn opencoder_llm::ChatStream> =
        std::sync::Arc::new(MockChatClient::new());
    let mut config = base_cfg;
    let mut model_label = String::new();
    let mut context_limit = 0u64;
    let mut frame_ms = 25u64;
    let mut frame_ticker = tokio::time::interval(std::time::Duration::from_millis(frame_ms));
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<crate::worker::UiCmd>(64);
    let mut chat = crate::chat::ChatView::default();

    let k = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::empty(),
    );
    let flow = handle_model_outcome(
        &mut model_menu,
        k,
        &mut client,
        &mut config,
        &mut model_label,
        &mut context_limit,
        &mut frame_ms,
        &mut frame_ticker,
        &cmd_tx,
        &mut chat,
        workdir,
    )
    .await;

    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(model_menu.is_none(), "modal should close on Save");

    // Collect all marker blocks; expect at least the red error marker and the
    // green "saved" marker.
    let markers: Vec<&[ratatui::text::Line]> = chat
        .blocks
        .iter()
        .filter_map(|b| match b {
            ChatBlock::Marker(lines) => Some(lines.as_slice()),
            _ => None,
        })
        .collect();
    assert!(
        markers.len() >= 2,
        "expected at least 2 markers (error + saved), got {}",
        markers.len()
    );

    // The first marker is the red error; it must mention "client build failed".
    let error_text: String = markers[0]
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect();
    assert!(
        error_text.contains("client build failed"),
        "expected 'client build failed' in error marker, got: {error_text}"
    );

    // A `ReloadConfig` command must have been sent regardless of the error.
    let cmd = cmd_rx.recv().await.expect("ReloadConfig should be sent");
    assert!(matches!(cmd, crate::worker::UiCmd::ReloadConfig(_)));
}

/// `resolve_endpoint` fails when no api_key is available (neither the merged
/// config nor `OPENAI_API_KEY` provides one) → the "endpoint resolve failed"
/// red marker is pushed. HOME is redirected to a temp dir so the global config
/// candidates can't smuggle in an api_key.
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn handle_model_outcome_endpoint_resolve_failure_pushes_red_marker() {
    use crate::chat::ChatBlock;
    use crate::model_menu::{ConfigField, ConfigForm, ModelMenu};
    use opencoder_core::Config;
    use opencoder_llm::MockChatClient;

    let _guard = HOME_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // Redirect HOME to a temp dir so no global config can supply an api_key,
    // and clear any inherited `OPENAI_API_KEY`. RAII guards guarantee
    // restoration even if an assertion panics mid-`await`.
    let tmp = tempfile::tempdir().unwrap();
    let _home_guard = EnvGuard::set("HOME", tmp.path());
    let _key_guard = EnvGuard::remove("OPENAI_API_KEY");

    let workdir = tmp.path();

    // Pre-write a config with no api_key — `resolve_endpoint` will fail.
    let config_json = serde_json::json!({
        "model": "openai/no-key-model"
    });
    std::fs::write(workdir.join("opencoder.json"), config_json.to_string()).unwrap();

    let base_cfg = Config::default();
    let mut form = ConfigForm::new(&base_cfg);
    form.threshold_input = "80000".into();
    form.focus = ConfigField::Save;
    let mut model_menu = Some(ModelMenu::Config(form));

    let mut client: std::sync::Arc<dyn opencoder_llm::ChatStream> =
        std::sync::Arc::new(MockChatClient::new());
    let mut config = base_cfg;
    let mut model_label = String::new();
    let mut context_limit = 0u64;
    let mut frame_ms = 25u64;
    let mut frame_ticker = tokio::time::interval(std::time::Duration::from_millis(frame_ms));
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<crate::worker::UiCmd>(64);
    let mut chat = crate::chat::ChatView::default();

    let k = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::empty(),
    );
    let flow = handle_model_outcome(
        &mut model_menu,
        k,
        &mut client,
        &mut config,
        &mut model_label,
        &mut context_limit,
        &mut frame_ms,
        &mut frame_ticker,
        &cmd_tx,
        &mut chat,
        workdir,
    )
    .await;

    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(model_menu.is_none(), "modal should close on Save");

    let markers: Vec<&[ratatui::text::Line]> = chat
        .blocks
        .iter()
        .filter_map(|b| match b {
            ChatBlock::Marker(lines) => Some(lines.as_slice()),
            _ => None,
        })
        .collect();
    assert!(
        markers.len() >= 2,
        "expected at least 2 markers (error + saved), got {}",
        markers.len()
    );

    let error_text: String = markers[0]
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect();
    assert!(
        error_text.contains("endpoint resolve failed"),
        "expected 'endpoint resolve failed' in error marker, got: {error_text}"
    );

    let cmd = cmd_rx.recv().await.expect("ReloadConfig should be sent");
    assert!(matches!(cmd, crate::worker::UiCmd::ReloadConfig(_)));
}
