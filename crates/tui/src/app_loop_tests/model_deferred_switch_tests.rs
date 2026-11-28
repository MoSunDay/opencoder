//! Mid-turn `/model` selection on the UI side: the ack-free push onto the
//! session's `ModelSwitchQueue`, the `SessionEvent::ModelOverrideApplied`
//! fold that commits the display once the runner applied the pick, and the
//! refusal / error markers. The runner half (apply after the current LLM
//! round, `ModelSwitch` then `ModelOverrideApplied`) is covered by
//! `crates/session/tests/model_switch_mid_turn.rs`.
//!
//! Contract: while a turn runs the status bar keeps showing the model the
//! in-flight request uses; only the applied pick (or its failure) changes it.

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::{Config, ConfigMode};
use opencoder_session::{ModelSwitchQueue, SessionEvent};
use opencoder_store::{Store, TursoStore};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TryRecvError;
use tokio_util::sync::CancellationToken;

use super::config_busy_gate_tests::{config, press};
use super::HOME_TEST_LOCK;
use crate::app::app_loop::*;
use crate::chat::{ChatBlock, ChatView};
use crate::model_menu::{ModelMenu, ProviderList};
use crate::model_session_switch::{
    commit_model_override, commit_session_switch, queue_session_switch,
};
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

/// Fold one `ModelOverrideApplied` through the real `fold_ui_events`, exactly
/// like the app loop does when the runner finished an LLM round. Returns
/// nothing else was drained or quit: the fold is a pure display commit.
async fn fold_applied(
    chat: &mut ChatView,
    config: &Config,
    mode: &str,
    model: &str,
    error: Option<&str>,
) {
    let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let (_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);
    let mut queue_items = Vec::new();
    let mut plan_skill_active = false;
    let mut admit = crate::queue_admitter::AdmitUiState::default();
    let mut running = true;
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
        Some(UiEvent::Session(SessionEvent::ModelOverrideApplied {
            mode: mode.to_string(),
            model: model.to_string(),
            error: error.map(str::to_string),
        })),
        chat,
        config,
        &store,
        "mid-turn-model-session",
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
        running && !drain_pending,
        "an applied pick is not a turn boundary"
    );
    assert!(matches!(evt_rx.try_recv(), Err(TryRecvError::Empty)));
    assert!(
        matches!(cmd_rx.try_recv(), Err(TryRecvError::Empty)),
        "the fold sends no worker command"
    );
}

/// The full UI round trip for a mid-turn Act pick: `Enter` arms the prompt,
/// `n` queues the pick on the session queue (no worker command, display
/// untouched, "queued" marker), and the runner's `ModelOverrideApplied`
/// commits the display with a "switched" marker — same end state as the idle
/// `ApplySession` path.
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn queued_selection_commits_display_only_after_model_override_applied() {
    let _guard = HOME_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();
    let queue = ModelSwitchQueue::default();
    let chat = ChatView {
        agent: "act".into(),
        ..Default::default()
    };
    let list = Some(ModelMenu::List(ProviderList::new(&config())));

    let (flow, menu, _, chat, _, command) =
        press(true, enter(), list, chat, workdir, false, &queue).await;
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(menu.is_some(), "arming the prompt is local");
    assert!(command.is_none());
    assert!(markers(&chat).is_empty(), "no marker for a local key");

    let n_key = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::empty());
    let (flow, menu, outer, mut chat, client_changed, command) =
        press(true, n_key, menu, chat, workdir, false, &queue).await;
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(menu.is_none(), "the consumed prompt closes");
    assert!(
        command.is_none(),
        "the pick travels on the queue, not the worker"
    );
    assert!(!client_changed, "a model-only pick keeps the client");
    assert_eq!(
        opencoder_session::drain_model_switches(&queue),
        vec![(ConfigMode::Act, "deepseek/deepseek-chat".to_string())]
    );
    assert!(
        chat.model_overrides.is_none(),
        "display commits only on ModelOverrideApplied"
    );
    let queued = markers(&chat).join("\n");
    assert!(queued.contains("queued \u{2192}"), "{queued}");
    assert!(
        queued.contains("(applies when the current LLM round ends)"),
        "{queued}"
    );
    assert!(!queued.contains("switched"), "{queued}");

    fold_applied(&mut chat, &outer, "act", "deepseek/deepseek-chat", None).await;

    let overrides = chat.model_overrides.as_ref().expect("committed");
    assert_eq!(
        overrides.get(ConfigMode::Act),
        Some("deepseek/deepseek-chat"),
        "the pick lands in the Act slot"
    );
    // Legacy-row conversion (same rule as `SessionState::update_mode_model`):
    // the first selection pins the other modes to the previous base.
    assert_eq!(
        overrides.get(ConfigMode::Plan),
        Some(config().model.as_str()),
        "unselected Plan is pinned to the legacy base"
    );
    assert_eq!(chat.shared_model, config().model, "Shared is never a pick");
    assert_eq!(
        chat.model, "deepseek/deepseek-chat",
        "Act chat follows the pick"
    );
    let text = markers(&chat).join("\n");
    assert!(
        text.contains("[/model] Act switched (session only) \u{2192} deepseek/deepseek-chat"),
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

/// A plan pick lands in the Plan slot; an Act chat keeps its own model while
/// the display copy still records the Plan override.
#[tokio::test]
async fn plan_pick_only_moves_the_plan_slot() {
    let config = config();
    let mut chat = ChatView {
        agent: "act".into(),
        ..Default::default()
    };
    fold_applied(&mut chat, &config, "plan", "two/plan", None).await;
    let overrides = chat.model_overrides.as_ref().expect("committed");
    assert_eq!(overrides.get(ConfigMode::Plan), Some("two/plan"));
    assert_eq!(overrides.get(ConfigMode::Act), Some(config.model.as_str()));
    assert_eq!(chat.model, config.model, "Act chat keeps the Act model");
    assert_eq!(chat.shared_model, config.model);
}

/// A refused pick keeps every display field on the previous model and
/// explains that the runtime is unchanged.
#[tokio::test]
async fn refused_pick_keeps_display_on_previous_model() {
    let config = config();
    let mut chat = ChatView {
        agent: "act".into(),
        model: config.model.clone(),
        shared_model: config.model.clone(),
        ..Default::default()
    };

    fold_applied(
        &mut chat,
        &config,
        "act",
        "two/act",
        Some("injected override failure"),
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

/// `/model` has no Shared scope: a Shared (or unknown) wire key never reaches
/// the display overrides, it is only reported.
#[test]
fn pick_outside_act_and_plan_is_reported_not_committed() {
    for key in ["shared", "team"] {
        let config = config();
        let mut chat = ChatView::default();
        commit_model_override(&mut chat, &config, key, "two/other", None);
        assert!(chat.model_overrides.is_none(), "{key} must not commit");
        let text = markers(&chat).join("\n");
        assert!(text.contains(&format!("`{key}`")), "{text}");
        assert!(text.contains("ignored"), "{text}");
        assert!(!text.contains("switched"), "{text}");
    }
}

/// Provider edits rebuild the client and belong to the idle `/model` path:
/// mid-turn they are refused BEFORE anything reaches the session queue.
#[tokio::test]
async fn provider_patch_while_running_is_refused_before_queueing() {
    let mut chat = ChatView::default();
    let queue = ModelSwitchQueue::default();

    queue_session_switch(
        serde_json::json!({"providers": {"deepseek": null}}),
        &queue,
        &mut chat,
    );

    assert_eq!(
        opencoder_session::pending_model_switches(&queue),
        0,
        "provider edits are never queued mid-turn"
    );
    assert!(chat.model_overrides.is_none());
    let text = markers(&chat).join("\n");
    assert!(text.contains("Provider changes apply when idle"), "{text}");
}

/// Only an Act/Plan patch carries a session pick: the top-level Shared
/// default (onboarding / `/config`) is not a `/model` selection.
#[tokio::test]
async fn patch_without_act_or_plan_model_is_refused() {
    for json in [
        serde_json::json!({"model": "openai/after"}),
        serde_json::json!({"modes": {"shared": {"model": "openai/shared"}}}),
        serde_json::json!({"fps": 10}),
    ] {
        let mut chat = ChatView::default();
        let queue = ModelSwitchQueue::default();
        queue_session_switch(json.clone(), &queue, &mut chat);
        assert_eq!(
            opencoder_session::pending_model_switches(&queue),
            0,
            "{json} must not be queued"
        );
        assert!(chat.model_overrides.is_none());
        let text = markers(&chat).join("\n");
        assert!(text.contains("no model selection"), "{text}");
        assert!(!text.contains("queued \u{2192}"), "{text}");
    }
}

/// Several picks made during one long request keep their submission order;
/// the runner applies (and reports) them in that order.
#[test]
fn queued_picks_keep_submission_order() {
    let mut chat = ChatView::default();
    let queue = ModelSwitchQueue::default();
    let picks = [
        (
            serde_json::json!({"modes":{"plan":{"model":"openai/plan"}}}),
            ConfigMode::Plan,
            "openai/plan",
        ),
        (
            serde_json::json!({"modes":{"act":{"model":"openai/act"}}}),
            ConfigMode::Act,
            "openai/act",
        ),
        (
            serde_json::json!({"modes":{"plan":{"model":"openai/plan-2"}}}),
            ConfigMode::Plan,
            "openai/plan-2",
        ),
    ];
    for (json, _, _) in &picks {
        queue_session_switch(json.clone(), &queue, &mut chat);
    }
    let expected: Vec<(ConfigMode, String)> = picks
        .iter()
        .map(|(_, mode, model)| (*mode, (*model).to_string()))
        .collect();
    assert_eq!(
        opencoder_session::pending_model_switches(&queue),
        expected.len(),
        "nothing is coalesced: the runner drains in order"
    );
    assert_eq!(opencoder_session::drain_model_switches(&queue), expected);
    assert_eq!(opencoder_session::pending_model_switches(&queue), 0);
    assert_eq!(
        markers(&chat).len(),
        expected.len(),
        "one queued marker per pick"
    );
    assert!(chat.model_overrides.is_none(), "queueing never commits");
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
        ConfigMode::Act,
        "openai/act-pick",
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
