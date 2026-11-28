//! Busy-turn refusal for the worker-acknowledged config menus (`/config`,
//! `/env`) and the ack-free session-only `/model` pick.
//!
//! The local worker runs a whole LLM turn inside one `process_cmd` and only
//! drains (and acks) config commands between turns. Awaiting that ack from
//! the single UI event loop froze the TUI until the turn ended — and
//! deadlocked outright once the event forwarder's bounded queue filled. The
//! dispatcher gate (`gate_config_menu`) keeps `/config` and `/env` closed
//! while a turn runs (see `slash_action_tests`); `handle_model_outcome(..,
//! running)` and `drain_pending_env(.., running)` are the safety net for a
//! turn that started while a menu was already open. `/model` is the
//! exception: its session-only selection is pushed onto the session's
//! `ModelSwitchQueue` (never the worker command channel), the runner applies
//! it after the current LLM round and the display commits only on the
//! resulting `SessionEvent::ModelOverrideApplied` (see
//! `model_deferred_switch_tests`).

use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc::error::TryRecvError;

use super::HOME_TEST_LOCK;
use crate::app::app_loop::*;
use crate::chat::{ChatBlock, ChatView};
use crate::command::EnvCommand;
use crate::model_menu::{EnvMenu, ModelMenu, ModelOutcome, ProviderList};
use crate::worker::UiCmd;
use opencoder_core::{Config, ConfigMode, ProviderConfig};
use opencoder_llm::{ChatStream, MockChatClient};

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

/// Only the three worker-acknowledged outcomes carry a command label; the
/// local ones (idle / cancel / quit) must stay usable mid-turn (Ctrl+D).
#[test]
fn worker_outcome_label_marks_only_worker_acked_outcomes() {
    assert_eq!(worker_outcome_label(&ModelOutcome::Idle), None);
    assert_eq!(worker_outcome_label(&ModelOutcome::Cancel), None);
    assert_eq!(worker_outcome_label(&ModelOutcome::Quit), None);
    assert_eq!(
        worker_outcome_label(&ModelOutcome::Save(serde_json::json!({}))),
        Some("/config")
    );
    assert_eq!(
        worker_outcome_label(&ModelOutcome::SaveSessionOnly(serde_json::json!({}))),
        Some("/model")
    );
    assert_eq!(
        worker_outcome_label(&ModelOutcome::ReloadEnv(
            Box::default(),
            "activated Env `demo`".into()
        )),
        Some("/env")
    );
}

/// Two custom providers so the provider list has an entry to switch to.
pub(super) fn config() -> Config {
    let mut config = Config {
        model: "openai/gpt-4o-mini".to_string(),
        provider: ProviderConfig {
            api_key: Some("placeholder".to_string()),
            ..Default::default()
        },
        ..Config::default()
    };
    config.providers.insert(
        "deepseek".to_string(),
        ProviderConfig {
            base_url: "https://example.invalid/v1".to_string(),
            api_key: Some("placeholder".to_string()),
            model: Some("deepseek-chat".to_string()),
            ..Default::default()
        },
    );
    config
}

/// One `handle_model_outcome` keystroke against a fresh runtime. Returns
/// `(flow, menu, config, chat, client changed, worker command)`. With
/// `expect_command` the worker side is driven concurrently (the command is
/// acknowledged with `Ok(())` so an idle dispatch completes); otherwise the
/// command channel is read BEFORE the sender drops, so "nothing was sent" is
/// distinguishable from "channel closed". `queue` is the session's model
/// switch queue the mid-turn path pushes onto.
#[allow(clippy::too_many_arguments)]
pub(super) async fn press(
    running: bool,
    key: KeyEvent,
    menu: Option<ModelMenu>,
    chat: ChatView,
    workdir: &std::path::Path,
    expect_command: bool,
    queue: &opencoder_session::ModelSwitchQueue,
) -> (
    LoopFlow,
    Option<ModelMenu>,
    Config,
    ChatView,
    bool,
    Option<UiCmd>,
) {
    let mut config = config();
    let mut menu = menu;
    let mut client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
    let client_before = Arc::as_ptr(&client);
    let mut compaction_threshold = config.compaction.context_threshold;
    let mut context_limit = config.context_limit();
    let mut frame_ms = 25u64;
    let mut frame_ticker = tokio::time::interval(Duration::from_millis(frame_ms));
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(64);
    let mut chat = chat;

    let outcome = handle_model_outcome(
        &mut menu,
        key,
        &mut client,
        &mut config,
        &mut compaction_threshold,
        &mut context_limit,
        &mut frame_ms,
        &mut frame_ticker,
        &cmd_tx,
        queue,
        &mut chat,
        workdir,
        None,
        running,
    );
    let (flow, command) = if expect_command {
        let (flow, command) = tokio::join!(outcome, async {
            let command = cmd_rx.recv().await.expect("worker command");
            match &command {
                UiCmd::ApplySession { ack, .. } | UiCmd::ReloadEnv { ack, .. } => {
                    ack.send(Ok(())).expect("completion receiver");
                }
                UiCmd::ReloadConfig { ack, .. } => {
                    ack.as_ref()
                        .expect("correlated reload")
                        .send(Ok(()))
                        .unwrap();
                }
                other => panic!("unexpected command: {other:?}"),
            }
            command
        });
        (flow, Some(command))
    } else {
        let flow = outcome.await;
        let command = match cmd_rx.try_recv() {
            Ok(command) => Some(command),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => panic!("sender dropped before the read"),
        };
        (flow, command)
    };
    (
        flow,
        menu,
        config,
        chat,
        !std::ptr::addr_eq(client_before, Arc::as_ptr(&client)),
        command,
    )
}

/// Enter arms the save-scope prompt (a local keystroke — still allowed
/// mid-turn); the `n` that confirms it never touches the worker command
/// channel: the session-only pick lands on the `ModelSwitchQueue`, outer
/// config / client / display selection stay unchanged until the runner's
/// `ModelOverrideApplied`, and the marker says when the switch applies.
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn session_only_switch_while_running_is_queued_without_worker_command() {
    let _guard = HOME_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();
    let queue = opencoder_session::ModelSwitchQueue::default();
    let list = Some(ModelMenu::List(ProviderList::new(&config())));

    let (flow, menu, _, chat, _, command) = press(
        true,
        enter(),
        list,
        ChatView::default(),
        workdir,
        false,
        &queue,
    )
    .await;
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(
        menu.is_some(),
        "arming the prompt is local and stays allowed"
    );
    assert!(command.is_none());
    assert!(markers(&chat).is_empty(), "no busy marker for a local key");

    let n_key = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::empty());
    let (flow, menu, outer, chat, client_changed, command) =
        press(true, n_key, menu, chat, workdir, false, &queue).await;
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(menu.is_none(), "the consumed prompt closes");
    assert!(
        command.is_none(),
        "mid-turn nothing reaches the serial worker channel: {command:?}"
    );
    assert_eq!(
        opencoder_session::drain_model_switches(&queue),
        vec![(ConfigMode::Act, "deepseek/deepseek-chat".to_string())],
        "the Act pick is queued for the next LLM round boundary"
    );
    assert_eq!(outer.model, "openai/gpt-4o-mini", "outer config untouched");
    assert!(
        chat.model_overrides.is_none(),
        "the display commits only on ModelOverrideApplied"
    );
    assert!(!client_changed, "client untouched");
    let text = markers(&chat).join("\n");
    assert!(text.contains("queued \u{2192}"), "{text}");
    assert!(
        text.contains("(applies when the current LLM round ends)"),
        "{text}"
    );
    assert!(text.contains("[/model] Act"), "{text}");
    assert!(!text.contains("Shared"), "{text}");
    assert!(!text.contains("switched"), "{text}");
    assert!(!text.contains("busy"), "{text}");

    // Idle again: the very same keystrokes go through to the worker, and the
    // queue stays untouched.
    let list = Some(ModelMenu::List(ProviderList::new(&config())));
    let (flow, menu, _, chat, _, command) =
        press(false, enter(), list, chat, workdir, false, &queue).await;
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(menu.is_some(), "idle: arming the prompt");
    assert!(command.is_none());
    let (flow, menu, _, chat, _, command) =
        press(false, n_key, menu, chat, workdir, true, &queue).await;
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(menu.is_none());
    assert!(
        matches!(
            &command,
            Some(UiCmd::ApplySession {
                selection: Some((ConfigMode::Act, model)),
                ..
            }) if model == "deepseek/deepseek-chat"
        ),
        "idle dispatch sends the Act ApplySession: {command:?}"
    );
    assert_eq!(
        opencoder_session::pending_model_switches(&queue),
        0,
        "idle picks never use the queue"
    );
    assert!(
        markers(&chat)
            .iter()
            .any(|line| line.contains("Act switched (session only)")),
        "idle dispatch reports the switch: {:?}",
        markers(&chat)
    );
}

/// The persisted `/config` save is refused BEFORE its disk write: the file
/// keeps the pre-save model, no `ReloadConfig` is sent and the marker names
/// `/config`.
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn shared_save_while_running_is_refused_before_disk_write() {
    let _guard = HOME_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let _isolation = opencoder_core::scoped_config_home(tmp.path().join("home"));
    let workdir = tmp.path().join("project");
    std::fs::create_dir_all(&workdir).unwrap();
    std::fs::write(
        workdir.join("opencoder.json"),
        serde_json::to_string(&config()).unwrap(),
    )
    .unwrap();
    let mut list = ProviderList::new(&config());
    list.confirm_save_default = Some(serde_json::json!({"model": "openai/after"}));
    let menu = Some(ModelMenu::List(list));

    let (flow, menu, outer, chat, client_changed, command) = press(
        true,
        enter(),
        menu,
        ChatView::default(),
        &workdir,
        false,
        &Default::default(),
    )
    .await;
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(menu.is_none(), "the consumed confirm dialog closes");
    assert!(
        command.is_none(),
        "no ReloadConfig while running: {command:?}"
    );
    assert_eq!(
        Config::load(&workdir).unwrap().model,
        "openai/gpt-4o-mini",
        "disk untouched"
    );
    assert_eq!(outer.model, "openai/gpt-4o-mini");
    assert!(!client_changed);
    let text = markers(&chat).join("\n");
    assert!(text.contains("[/config] busy"), "{text}");
    assert!(!text.contains("saved"), "{text}");
}

/// The loop-top Env drain is not a keystroke: while a turn runs it neither
/// executes the pending command, sends anything nor posts a marker — the
/// command stays pending for the first idle iteration.
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn drain_pending_env_while_running_keeps_command_pending() {
    let _guard = HOME_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let _isolation = opencoder_core::scoped_config_home(tmp.path().join("home"));
    let workdir = tmp.path();
    let mut config = config();
    let mut menu = Some(ModelMenu::Env(EnvMenu::with_command(
        workdir,
        EnvCommand::Activate("demo".into()),
    )));
    let mut client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
    let mut compaction_threshold = config.compaction.context_threshold;
    let mut context_limit = config.context_limit();
    let mut frame_ms = 25u64;
    let mut frame_ticker = tokio::time::interval(Duration::from_millis(frame_ms));
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(4);
    let mut chat = ChatView::default();

    let flow = drain_pending_env(
        &mut menu,
        &mut client,
        &mut config,
        &mut compaction_threshold,
        &mut context_limit,
        &mut frame_ms,
        &mut frame_ticker,
        &cmd_tx,
        &Default::default(),
        &mut chat,
        workdir,
        None,
        true,
    )
    .await;

    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(
        matches!(&menu, Some(ModelMenu::Env(m)) if m.has_pending()),
        "the Env command stays pending"
    );
    assert!(
        matches!(cmd_rx.try_recv(), Err(TryRecvError::Empty)),
        "no ReloadEnv / ReloadConfig while running"
    );
    assert!(markers(&chat).is_empty(), "no busy marker for the drain");
}

/// The Env menu's pending command executes (disk mutation) inside the key
/// handler — while a turn runs it is refused BEFORE that: the command stays
/// pending for the idle drain, nothing reaches the worker and the marker
/// names `/env`. (Without the pre-check the handler would have consumed the
/// command and reported an activation error instead.)
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn env_pending_command_while_running_is_refused_before_execution() {
    let _guard = HOME_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let _isolation = opencoder_core::scoped_config_home(tmp.path().join("home"));
    let workdir = tmp.path();
    let menu = Some(ModelMenu::Env(EnvMenu::with_command(
        workdir,
        EnvCommand::Activate("demo".into()),
    )));

    let (flow, menu, _, chat, _, command) = press(
        true,
        enter(),
        menu,
        ChatView::default(),
        workdir,
        false,
        &Default::default(),
    )
    .await;

    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(
        matches!(&menu, Some(ModelMenu::Env(m)) if m.has_pending()),
        "the command must not be consumed while a turn runs"
    );
    assert!(
        command.is_none(),
        "no ReloadEnv / ReloadConfig: {command:?}"
    );
    let text = markers(&chat).join("\n");
    assert!(text.contains("[/env] busy"), "{text}");
}
