//! Deferred (mid-turn) `/model` selection on the UI side: queueing without
//! an ack, the `UiEvent::ModelCommit` fold that commits the display, and the
//! refusal / error markers. The worker half (`ApplySessionDeferred` applied
//! at the turn boundary, `ModelCommit` emitted after `ModelSwitch`) is
//! covered in `worker/tests_model_commands.rs` and `worker/tests_failures.rs`.
//!
//! Contract: while a turn runs the status bar keeps showing the model the
//! in-flight requests use; only the commit (or its failure) changes it.

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::{Config, ConfigMode};
use opencoder_store::{Store, TursoStore};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TryRecvError;
use tokio_util::sync::CancellationToken;

use super::config_busy_gate_tests::{config, press};
use super::HOME_TEST_LOCK;
use crate::app::app_loop::*;
use crate::chat::{ChatBlock, ChatView};
use crate::model_menu::{ModelMenu, ProviderList};
use crate::model_session_switch::{commit_session_switch, queue_session_switch};
use crate::worker::{UiCmd, UiEvent};

fn markers(chat: &ChatView) -> Vec<String> {
    chat.blocks
        .iter()
        .filter_map(|block| match block {
            ChatBlock::Marker(lines) => Some(
                lines
                    .iter()
                    .flat_map(|line| line.spans.iter())
                    .map(|span| span.content.as_ref())
                    .collect(),
            ),
            _ => None,
        })
        .collect()
}

fn enter() -> KeyEvent {
    KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())
}

/// Fold one `ModelCommit` through the real `fold_ui_events`, exactly like
/// the app loop does at the turn boundary. Returns nothing else was drained
/// or quit: the fold is a pure display commit.
async fn fold_commit(
    chat: &mut ChatView,
    config: &Config,
    selection: Option<(ConfigMode, String)>,
    result: Result<(), String>,
) {
    let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let (_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);
    let mut queue_items = Vec::new();
    let mut plan_skill_active = false;
    let mut admit = crate::queue_admitter::AdmitUiState::default();
    let mut running = false;
    let mut cancelled = false;
    let mut drain_pending = false;
    let mut skip_next_render = false;
    let mut follow = true;
    let mut cancel = CancellationToken::new();
    let mut turn_gen = 0u64;
    let mut notepad: Option<crate::notepad::NotepadView> = None;
    let mut question_menu: Option<crate::question_menu::QuestionMenu> = None;
    let question_hub = Arc::new(opencoder_session::QuestionHub::new());
    let mut reset = crate::reset_rebuild::ResetRebuildState::default();

    let flow = fold_ui_events(
        Some(UiEvent::ModelCommit { selection, result }),
        chat,
        config,
        &store,
        "deferred-model-session",
        &mut queue_items,
        &mut plan_skill_active,
        &mut admit,
        &mut running,
        &mut cancelled,
        &mut drain_pending,
        &mut skip_next_render,
        &mut follow,
        &cmd_tx,
        &mut cancel,
        &mut turn_gen,
        &mut evt_rx,
        &mut notepad,
        &mut question_menu,
        &question_hub,
        &mut reset,
    )
    .await;

    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(
        !running && !drain_pending,
        "a commit is not a turn boundary"
    );
    assert!(matches!(evt_rx.try_recv(), Err(TryRecvError::Empty)));
    assert!(
        matches!(cmd_rx.try_recv(), Err(TryRecvError::Empty)),
        "the fold sends no worker command"
    );
}

/// The full UI round trip for a mid-turn pick: `Enter` arms the prompt,
/// `n` queues `ApplySessionDeferred` (display untouched, "queued" marker),
/// and the worker's `ModelCommit` commits the display with a "switched"
/// marker — same end state as the idle `ApplySession` path.
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn queued_selection_commits_display_only_after_model_commit() {
    let _guard = HOME_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();
    let chat = ChatView {
        agent: "act".into(),
        ..Default::default()
    };
    let list = Some(ModelMenu::List(ProviderList::new(&config())));

    let (flow, menu, _, chat, _, command) = press(true, enter(), list, chat, workdir, false).await;
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(menu.is_some(), "arming the prompt is local");
    assert!(command.is_none());
    assert!(markers(&chat).is_empty(), "no marker for a local key");

    let n_key = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::empty());
    let (flow, menu, outer, mut chat, client_changed, command) =
        press(true, n_key, menu, chat, workdir, false).await;
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(menu.is_none(), "the consumed prompt closes");
    assert!(!client_changed, "a model-only pick keeps the client");
    let Some(UiCmd::ApplySessionDeferred { selection, .. }) = command else {
        panic!("expected a deferred queue, got {command:?}");
    };
    let selection = selection.expect("the prompt carries a model selection");
    let model = selection.1.clone();
    assert_eq!(selection.0, ConfigMode::Shared);
    assert!(
        chat.model_overrides.is_none(),
        "display commits only on ModelCommit"
    );
    let queued = markers(&chat).join("\n");
    assert!(queued.contains("queued \u{2192}"), "{queued}");
    assert!(
        queued.contains("(applies when the current turn ends)"),
        "{queued}"
    );
    assert!(!queued.contains("switched"), "{queued}");

    fold_commit(&mut chat, &outer, Some(selection), Ok(())).await;

    let overrides = chat.model_overrides.as_ref().expect("committed");
    assert_eq!(overrides.get(ConfigMode::Shared), Some(model.as_str()));
    assert_eq!(chat.shared_model, model, "Shared follows the commit");
    // Legacy-row conversion (same rule as `SessionState::update_mode_model`):
    // the first selection pins the unselected modes to the previous base, so
    // an Act chat keeps showing that base while Shared moves to the pick.
    assert_eq!(
        overrides.get(ConfigMode::Act),
        Some(config().model.as_str()),
        "unselected Act is pinned to the legacy base"
    );
    assert_eq!(chat.model, config().model, "Act chat keeps the Act model");
    let text = markers(&chat).join("\n");
    assert!(
        text.contains(&format!(" switched (session only) \u{2192} {model}")),
        "{text}"
    );
    assert!(
        text.contains("queued"),
        "the queued marker is history: {text}"
    );
    assert_eq!(
        outer.model, "openai/gpt-4o-mini",
        "session-only: no config write"
    );
}

/// A failed worker commit keeps every display field on the previous model
/// and explains that the runtime is unchanged.
#[tokio::test]
async fn failed_commit_keeps_display_on_previous_model() {
    let config = config();
    let mut chat = ChatView {
        agent: "act".into(),
        model: config.model.clone(),
        shared_model: config.model.clone(),
        ..Default::default()
    };

    fold_commit(
        &mut chat,
        &config,
        Some((ConfigMode::Act, "two/act".into())),
        Err("injected override failure".into()),
    )
    .await;

    assert!(chat.model_overrides.is_none(), "nothing committed");
    assert_eq!(chat.model, config.model);
    assert_eq!(chat.shared_model, config.model);
    let text = markers(&chat).join("\n");
    assert!(
        text.contains(
            "[/model] session-only change failed: injected override failure; runtime unchanged"
        ),
        "{text}"
    );
    assert!(!text.contains("switched"), "{text}");
}

/// Provider edits rebuild the client and belong to the idle `/model` path:
/// mid-turn they are refused BEFORE anything reaches the worker.
#[tokio::test]
async fn provider_patch_while_running_is_refused_before_queueing() {
    let config = config();
    let mut chat = ChatView::default();
    let (tx, mut rx) = mpsc::channel::<UiCmd>(4);

    queue_session_switch(
        serde_json::json!({"providers": {"deepseek": null}}),
        &config,
        &tx,
        &mut chat,
    );

    assert!(
        matches!(rx.try_recv(), Err(TryRecvError::Empty)),
        "provider edits are never queued mid-turn"
    );
    assert!(chat.model_overrides.is_none());
    let text = markers(&chat).join("\n");
    assert!(text.contains("Provider changes apply when idle"), "{text}");
    assert!(!text.contains("queued"), "{text}");
}

/// A patch without a model selection has nothing to queue.
#[tokio::test]
async fn selection_less_patch_is_refused_before_queueing() {
    let config = config();
    let mut chat = ChatView::default();
    let (tx, mut rx) = mpsc::channel::<UiCmd>(4);

    queue_session_switch(serde_json::json!({"modes": {}}), &config, &tx, &mut chat);

    assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    assert!(chat.model_overrides.is_none());
    let text = markers(&chat).join("\n");
    assert!(text.contains("contained no model selection"), "{text}");
}

/// The busy path uses `try_send`: a full command channel reports "not
/// queued" instead of parking the UI loop on a turn it is streaming.
#[tokio::test]
async fn full_command_channel_reports_not_queued_instead_of_blocking() {
    let config = config();
    let mut chat = ChatView::default();
    let (tx, _rx) = mpsc::channel::<UiCmd>(1);
    tx.try_send(UiCmd::Compact).unwrap();

    queue_session_switch(
        serde_json::json!({"model": "openai/after"}),
        &config,
        &tx,
        &mut chat,
    );

    assert!(chat.model_overrides.is_none());
    let text = markers(&chat).join("\n");
    assert!(text.contains("not queued"), "{text}");
    assert!(text.contains("runtime unchanged"), "{text}");
    assert!(!text.contains("queued \u{2192}"), "{text}");
}

/// `commit_session_switch` with no selection and success is a silent no-op
/// (the worker only reports this shape for provider-only patches).
#[test]
fn commit_without_selection_is_silent() {
    let config = config();
    let mut chat = ChatView::default();
    commit_session_switch(&mut chat, &config, None, Ok(()));
    assert!(chat.model_overrides.is_none());
    assert!(markers(&chat).is_empty());
}

/// `commit_session_switch` is also the direct display entry point: it needs
/// neither a client nor a worker channel, so the fold stays side-effect free
/// beyond the chat view itself.
#[test]
fn commit_display_matches_the_idle_switch_end_state() {
    let config = config();
    let mut chat = ChatView {
        agent: "act".into(),
        model: config.model.clone(),
        shared_model: config.model.clone(),
        ..Default::default()
    };
    commit_session_switch(
        &mut chat,
        &config,
        Some((ConfigMode::Act, "openai/act-pick".into())),
        Ok(()),
    );
    assert_eq!(
        chat.model_overrides
            .as_ref()
            .and_then(|o| o.get(ConfigMode::Act)),
        Some("openai/act-pick")
    );
    assert_eq!(chat.model, "openai/act-pick");
    assert_eq!(chat.shared_model, config.model, "Shared untouched");
    assert_eq!(
        markers(&chat),
        vec!["[/model] Act switched (session only) \u{2192} openai/act-pick".to_string()]
    );
}
