//! Tests for `app_loop` helpers — extracted to keep `app_loop.rs` under the
//! 800-line cap. Compiled as `#[cfg(test)] mod tests` via `#[path]`.

use super::*;
use crate::chat::ChatView;

// ----- Existing route_paste tests -----

/// No modal open + plain (non-file) text: the main-composer path inserts it
/// verbatim, advances the cursor, and returns `Proceed` (caller falls
/// through rather than `continue`).
#[test]
fn route_paste_into_main_composer_inserts_verbatim_text() {
    let mut model_menu: Option<ModelMenu> = None;
    let mut command_menu: Option<CommandMenu> = None;
    let mut input = String::new();
    let mut idx = 0usize;
    let mut pending_images: Vec<(String, String)> = Vec::new();
    let flow = route_paste(
        "plain text",
        false,
        false,
        &mut model_menu,
        &mut command_menu,
        &mut input,
        &mut idx,
        &mut pending_images,
        Path::new("."),
    );
    assert!(matches!(flow, LoopFlow::Proceed));
    assert_eq!(input, "plain text");
    assert_eq!(idx, "plain text".chars().count());
}

/// task picker open (no text field): the paste is swallowed — `Redraw` is
/// returned and the main composer stays untouched.
#[test]
fn route_paste_swallowed_when_task_picker_open() {
    let mut model_menu: Option<ModelMenu> = None;
    let mut command_menu: Option<CommandMenu> = None;
    let mut input = String::new();
    let mut idx = 0usize;
    let mut pending_images: Vec<(String, String)> = Vec::new();
    let flow = route_paste(
        "plain text",
        true,
        false,
        &mut model_menu,
        &mut command_menu,
        &mut input,
        &mut idx,
        &mut pending_images,
        Path::new("."),
    );
    assert!(matches!(flow, LoopFlow::Redraw));
    assert!(
        input.is_empty(),
        "main composer must be untouched when a modal swallows the paste"
    );
    assert_eq!(idx, 0);
}

/// cache-salt menu open: same modal-isolation contract — paste swallowed,
/// existing composer contents and cursor preserved.
#[test]
fn route_paste_swallowed_when_cache_salt_menu_open() {
    let mut model_menu: Option<ModelMenu> = None;
    let mut command_menu: Option<CommandMenu> = None;
    let mut input = String::from("kept");
    let mut idx = 2usize;
    let mut pending_images: Vec<(String, String)> = Vec::new();
    let flow = route_paste(
        "plain text",
        false,
        true,
        &mut model_menu,
        &mut command_menu,
        &mut input,
        &mut idx,
        &mut pending_images,
        Path::new("."),
    );
    assert!(matches!(flow, LoopFlow::Redraw));
    assert_eq!(input, "kept");
    assert_eq!(idx, 2);
}

// ----- plan→act handoff tests (P0 race-fix) -----

fn plan_view() -> ChatView {
    ChatView {
        agent: "plan".into(),
        plan_submitted: true,
        ..Default::default()
    }
}

/// Regression: plan→act while idle triggers the handoff immediately.
#[tokio::test]
async fn switch_plan_to_act_while_idle_triggers_handoff() {
    let mut chat = plan_view();
    let mut running = false;
    let mut follow = false;
    let mut input = "do it".to_string();
    let mut cursor_idx = 5;
    let mut mode_flash: Option<(String, u32)> = None;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut sys_tokens = 0u64;
    let workdir = Path::new(".");
    let active_skill_body: Option<String> = None;

    let outcome = handle_switch_agent(
        "act".into(),
        &mut chat,
        &mut running,
        &mut follow,
        &mut input,
        &mut cursor_idx,
        &mut mode_flash,
        0,
        &cmd_tx,
        &mut cancel,
        &mut sys_tokens,
        workdir,
        &active_skill_body,
    )
    .await;

    assert!(matches!(outcome, SwitchOutcome::Proceed));
    assert!(running);
    assert!(follow);
    // ResetCancel + SwitchAndStart
    assert!(matches!(cmd_rx.try_recv().unwrap(), UiCmd::ResetCancel(_)));
    match cmd_rx.try_recv().unwrap() {
        UiCmd::SwitchAndStart(ref n, ref extra) => {
            assert_eq!(n, "act");
            assert_eq!(extra, "do it");
        }
        _ => panic!("expected SwitchAndStart"),
    }
}

/// Regression for the removal of deferred handoff: plan→act Shift+Tab while
/// the plan turn is running is now a complete no-op — no command sent, input
/// untouched, running stays true, and a flash hint is shown.
#[tokio::test]
async fn switch_plan_to_act_while_running_is_noop() {
    let mut chat = plan_view();
    let mut running = true;
    let mut follow = true;
    let mut input = "do not lose me".to_string();
    let mut cursor_idx = 14;
    let mut mode_flash: Option<(String, u32)> = None;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut sys_tokens = 0u64;
    let workdir = Path::new(".");
    let active_skill_body: Option<String> = None;

    let outcome = handle_switch_agent(
        "act".into(),
        &mut chat,
        &mut running,
        &mut follow,
        &mut input,
        &mut cursor_idx,
        &mut mode_flash,
        0,
        &cmd_tx,
        &mut cancel,
        &mut sys_tokens,
        workdir,
        &active_skill_body,
    )
    .await;

    assert!(matches!(outcome, SwitchOutcome::Proceed));
    assert!(
        cmd_rx.try_recv().is_err(),
        "no command should be sent while running"
    );
    assert!(running, "running must stay true (plan turn still active)");
    assert_eq!(input, "do not lose me", "input must be untouched on no-op");
    assert_eq!(cursor_idx, 14, "cursor must be untouched on no-op");
    assert!(
        mode_flash
            .as_ref()
            .map(|(t, _)| t.contains("running"))
            .unwrap_or(false),
        "mode flash should hint that plan is running; got {:?}",
        mode_flash
    );
}

/// plan→act without a submitted plan is a pure switch (no handoff).
#[tokio::test]
async fn switch_plan_to_act_unsubmitted_is_pure_switch() {
    let mut chat = ChatView {
        agent: "plan".into(),
        plan_submitted: false,
        ..Default::default()
    };
    let mut running = false;
    let mut follow = false;
    let mut input = String::new();
    let mut cursor_idx = 0;
    let mut mode_flash: Option<(String, u32)> = None;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut sys_tokens = 0u64;
    let workdir = Path::new(".");
    let active_skill_body: Option<String> = None;

    let outcome = handle_switch_agent(
        "act".into(),
        &mut chat,
        &mut running,
        &mut follow,
        &mut input,
        &mut cursor_idx,
        &mut mode_flash,
        0,
        &cmd_tx,
        &mut cancel,
        &mut sys_tokens,
        workdir,
        &active_skill_body,
    )
    .await;

    assert!(matches!(outcome, SwitchOutcome::Proceed));
    assert!(!running);
    match cmd_rx.try_recv().unwrap() {
        UiCmd::SwitchAgent(ref n) => assert_eq!(n, "act"),
        _ => panic!("expected SwitchAgent"),
    }
}

// ----- fold_ui_events P0/P1 tests -----

use opencoder_core::Message;
use opencoder_session::SessionEvent;
use opencoder_store::{LibsqlStore, SessionMeta};

/// P1 fix: TranscriptReset (compaction) must NOT reset plan_submitted to false.
#[tokio::test]
async fn fold_transcript_reset_preserves_plan_submitted() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    // Create the session so replay_into_chat's store queries succeed.
    store
        .create_session(&SessionMeta {
            id: "p1-test".into(),
            agent: Some("plan".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    let mut chat = ChatView {
        agent: "plan".into(),
        plan_submitted: true,
        ..Default::default()
    };
    let messages = vec![Message::user("u1", "compacted summary")];
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut running = false;
    let mut cancelled = false;
    let mut drain_pending = false;
    let mut skip_next_render = false;
    let mut follow = true;
    let (cmd_tx, _cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);

    let _flow = fold_ui_events(
        Some(UiEvent::Session(SessionEvent::TranscriptReset(messages))),
        &mut chat,
        &store,
        "p1-test",
        &mut queue_items,
        &mut running,
        &mut cancelled,
        &mut drain_pending,
        &mut skip_next_render,
        &mut follow,
        &cmd_tx,
        &mut cancel,
        &mut evt_rx,
    )
    .await;

    assert!(
        chat.plan_submitted,
        "plan_submitted must survive TranscriptReset (compaction); \
         this is the P1 regression — without the fix, the replay would \
         reset it to false"
    );
}

// ----- handle_model_outcome Err-branch tests -----
//
// `handle_model_outcome` walks the save→reload→resolve_endpoint→ChatClient::new
// chain; the last two steps can fail. Each failure path must push a red error
// marker into `chat`, then still send `UiCmd::ReloadConfig` and a green "saved"
// marker (the reload/saved markers are pushed unconditionally after the inner
// match — see `app_loop.rs`). These two tests pin the error-marker text and
// the ReloadConfig dispatch for each Err branch.

static HOME_LOCK_1: std::sync::Mutex<()> = std::sync::Mutex::new(());
static HOME_LOCK_2: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// RAII guard that restores an env var to its prior value on drop,
/// guaranteeing restoration even if a test assertion panics mid-`await`.
struct EnvGuard {
    key: &'static str,
    old: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let old = std::env::var_os(key);
        std::env::set_var(key, value);
        EnvGuard { key, old }
    }

    fn remove(key: &'static str) -> Self {
        let old = std::env::var_os(key);
        std::env::remove_var(key);
        EnvGuard { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}

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

    let _guard = HOME_LOCK_1.lock().unwrap_or_else(|e| e.into_inner());

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
    form.threshold = 80000; // ensure validation passes (>= 1000)
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

    let _guard = HOME_LOCK_2.lock().unwrap_or_else(|e| e.into_inner());

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
    form.threshold = 80000;
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

// ----- Done/Error queue_items clear tests -----
//
// Regression: `fold_ui_events`'s `Done | Error` handler used to
// unconditionally `queue_items.clear()`. On `Done` this is safe — the
// store queue is provably empty (claim_one_queued returned None before
// Done was emitted). On `Error` it is WRONG: the error path
// short-circuits run_loop before the idle boundary, so queued items may
// still be pending in the store. Wiping the in-memory mirror makes them
// invisible in the UI even though they would be consumed on the next
// drain. The fix only clears `queue_items` on `Done`.

/// Pre-populate `queue_items` with a couple of pending entries (as if a
/// steer was submitted while running, then the fresh drain errored) and
/// drive `fold_ui_events` with an `Error` event. The mirror must survive
/// — `running` flips off but `queue_items` stays intact.
#[tokio::test]
async fn fold_error_does_not_clear_queue_items() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView::default();
    let mut queue_items: Vec<(i64, String)> = vec![
        (10, "queued prompt A".into()),
        (11, "queued prompt B".into()),
    ];
    let mut running = true;
    let mut cancelled = false;
    let mut drain_pending = false;
    let mut skip_next_render = false;
    let mut follow = true;
    let (cmd_tx, _cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);

    let _flow = fold_ui_events(
        Some(UiEvent::Session(SessionEvent::Error(
            "llm api failure".into(),
        ))),
        &mut chat,
        &store,
        "test-session",
        &mut queue_items,
        &mut running,
        &mut cancelled,
        &mut drain_pending,
        &mut skip_next_render,
        &mut follow,
        &cmd_tx,
        &mut cancel,
        &mut evt_rx,
    )
    .await;

    assert!(
        !running,
        "running should flip false on Error (not cancelled, no drain pending)"
    );
    assert!(
        chat.steer_items.is_empty(),
        "steer_items should be cleared on Error"
    );
    assert_eq!(
        queue_items.len(),
        2,
        "queue_items must NOT be cleared on Error — items may still be \
         pending in the store and would be consumed on the next drain"
    );
    assert_eq!(queue_items[0].0, 10);
    assert_eq!(queue_items[1].0, 11);
}

/// Counterpart: on `Done` the store queue is provably empty
/// (claim_one_queued returned None before Done was emitted), so the
/// in-memory mirror should be wiped.
#[tokio::test]
async fn fold_done_clears_queue_items() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView::default();
    let mut queue_items: Vec<(i64, String)> = vec![
        (20, "queued prompt C".into()),
        (21, "queued prompt D".into()),
    ];
    let mut running = true;
    let mut cancelled = false;
    let mut drain_pending = false;
    let mut skip_next_render = false;
    let mut follow = true;
    let (cmd_tx, _cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);

    let _flow = fold_ui_events(
        Some(UiEvent::Session(SessionEvent::Done)),
        &mut chat,
        &store,
        "test-session",
        &mut queue_items,
        &mut running,
        &mut cancelled,
        &mut drain_pending,
        &mut skip_next_render,
        &mut follow,
        &cmd_tx,
        &mut cancel,
        &mut evt_rx,
    )
    .await;

    assert!(!running, "running should flip false on Done");
    assert!(
        chat.steer_items.is_empty(),
        "steer_items should be cleared on Done"
    );
    assert!(
        queue_items.is_empty(),
        "queue_items should be cleared on Done — store queue is provably empty"
    );
}

/// Safety: when the turn was cancelled (`cancelled=true`), neither
/// `Done` nor `Error` should touch `queue_items` — the event belongs to
/// a stale turn and items may belong to a fresh turn.
#[tokio::test]
async fn fold_error_when_cancelled_preserves_queue_items() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView::default();
    let mut queue_items: Vec<(i64, String)> = vec![(30, "queued after steer".into())];
    let mut running = true;
    let mut cancelled = true;
    let mut drain_pending = false;
    let mut skip_next_render = false;
    let mut follow = true;
    let (cmd_tx, _cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);

    let _flow = fold_ui_events(
        Some(UiEvent::Session(SessionEvent::Error("stale".into()))),
        &mut chat,
        &store,
        "test-session",
        &mut queue_items,
        &mut running,
        &mut cancelled,
        &mut drain_pending,
        &mut skip_next_render,
        &mut follow,
        &cmd_tx,
        &mut cancel,
        &mut evt_rx,
    )
    .await;

    assert!(
        running,
        "running must stay true when the event is from a cancelled turn"
    );
    assert!(!cancelled, "cancelled flag should be reset to false");
    assert_eq!(
        queue_items.len(),
        1,
        "queue_items must be untouched for a stale (cancelled) Error event"
    );
    assert_eq!(queue_items[0].0, 30);
}

// ----- Regression: status bar shows bare model id, not provider/model -----

/// The status bar's `status_model` must strip the `provider/` prefix so the
/// user sees `glm-5.2` rather than the full `bigmodel/glm-5.2`. This guards
/// against regressions where the raw `config.model` leaks through.
#[test]
fn compute_display_strips_provider_prefix_from_status_model() {
    use opencoder_core::Config;

    let chat = ChatView::default();
    let config = Config {
        model: "bigmodel/glm-5.2".to_string(),
        ..Config::default()
    };

    let ds = compute_display(&chat, None, 0, 0, &config, Path::new("."));

    assert_eq!(
        ds.status_model, "glm-5.2",
        "status bar must show only the model id without provider prefix"
    );
    assert!(
        !ds.status_model.contains('/'),
        "status_model must not contain the provider separator '/': got {}",
        ds.status_model
    );
}

/// With a reasoning-effort badge the prefix must still be stripped, yielding
/// e.g. "glm-5.2 ·high".
#[test]
fn compute_display_status_model_with_effort_strips_prefix() {
    use opencoder_core::Config;

    let chat = ChatView::default();
    let config = Config {
        model: "bigmodel/glm-5.2".to_string(),
        reasoning_effort: Some("high".to_string()),
        ..Config::default()
    };

    let ds = compute_display(&chat, None, 0, 0, &config, Path::new("."));

    assert_eq!(
        ds.status_model, "glm-5.2 \u{00b7}high",
        "status bar must show bare id plus effort badge; got: {}",
        ds.status_model
    );
}

