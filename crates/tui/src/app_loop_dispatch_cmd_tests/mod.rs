//! dispatch_command tests for the agent-switch popup path (`/act`,
//! `/sandbox`, `/act_clear_context`).
//!
//! Guards two behaviors after the plan/act dual-mode removal (all agent
//! switching flows through the control-command prompt, short-circuited by
//! the runner at idle):
//! 1. From idle, dispatching one of the three commands submits the
//!    control-command text as a prompt (`UiCmd::Prompt`) after the usual
//!    `ResetCancel` preamble.
//! 2. While a turn is running, the busy gate refuses with a marker and
//!    sends nothing.

use super::super::*;
use crate::chat::ChatBlock;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::Config;
use opencoder_store::LibsqlStore;

mod act_clear;

/// Build a CommandMenu filtered to the given query.
fn menu_for(query: &str) -> Option<CommandMenu> {
    let mut cm = CommandMenu::new();
    cm.paste(query);
    Some(cm)
}

/// Build a CommandMenu with query "act" and navigate past /compact (which
/// also matches "act" since "comp**act**") to select /act specifically.
fn menu_for_act() -> Option<CommandMenu> {
    let mut cm = CommandMenu::new();
    cm.paste("act"); // rows: [/compact, /act]
    cm.move_down(); // select /act (2nd match)
    Some(cm)
}

fn enter_key() -> KeyEvent {
    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
}

/// Drain the first real UiCmd from cmd_rx, skipping the ResetCancel preamble
/// that start_turn always sends first.
fn drain_cmd(cmd_rx: &mut mpsc::Receiver<UiCmd>) -> UiCmd {
    let first = cmd_rx.try_recv().unwrap();
    assert!(
        matches!(first, UiCmd::ResetCancel(_)),
        "expected ResetCancel preamble, got wrong variant"
    );
    cmd_rx.try_recv().unwrap()
}

/// Shared harness: dispatch the popup's highlighted command with Enter.
#[allow(clippy::too_many_arguments)]
async fn dispatch_popup(
    command_menu: &mut Option<CommandMenu>,
    chat: &mut ChatView,
    running: bool,
    agent: &str,
) -> (
    LoopFlow,
    mpsc::Receiver<UiCmd>,
    bool,
    Option<crate::clear_confirm::ClearConfirm>,
    Option<(String, u32)>,
    Vec<String>,
) {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut running = running;
    let mut follow = true;
    let mut task_picker = None;
    let mut model_menu = None;
    let mut mcp_menu: Option<crate::mcp_menu::McpMenu> = None;
    let mut cache_salt_menu = None;
    let mut input = String::new();
    let mut cursor_idx = 0usize;
    let mut config = Config::default();
    let workdir = std::path::Path::new(".");
    let mut mode_flash: Option<(String, u32)> = None;
    let mut sys_tokens = 0u64;
    let (cmd_tx, cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut clear_confirm: Option<crate::clear_confirm::ClearConfirm> = None;

    let flow = dispatch_command(
        command_menu,
        enter_key(),
        &cmd_tx,
        &mut cancel,
        chat,
        &mut running,
        &mut follow,
        &store,
        "test",
        &mut task_picker,
        &mut model_menu,
        &mut mcp_menu,
        &mut None,
        &mut None,
        &mut None,
        &mut None,
        &mut cache_salt_menu,
        &mut None,
        agent,
        &mut input,
        &mut cursor_idx,
        &mut config,
        workdir,
        &mut mode_flash,
        0,
        &mut sys_tokens,
        &mut None,
        &mut None,
        &mut clear_confirm,
    )
    .await;
    let chat_markers = marker_texts(chat);
    (flow, cmd_rx, running, clear_confirm, mode_flash, chat_markers)
}

/// Marker lines currently in the chat, as flat strings (assert helper).
fn marker_texts(chat: &ChatView) -> Vec<String> {
    chat.blocks
        .iter()
        .filter_map(|b| match b {
            ChatBlock::Marker(lines) => Some(lines.iter().map(|l| l.to_string()).collect::<Vec<_>>()),
            _ => None,
        })
        .flatten()
        .collect()
}

/// `/sandbox` from idle submits the control-command prompt (regardless of
/// the previous agent — switching is now prompt-driven only).
#[tokio::test]
async fn slash_sandbox_from_idle_submits_prompt() {
    for prev_agent in ["act", "sandbox", ""] {
        let mut chat = ChatView {
            agent: prev_agent.into(),
            ..Default::default()
        };
        let mut menu = menu_for("sandbox");
        let (flow, mut cmd_rx, running, ..) = dispatch_popup(&mut menu, &mut chat, false, "act").await;
        assert!(matches!(flow, LoopFlow::Proceed));
        assert!(running, "the switch turn starts immediately from idle");
        match drain_cmd(&mut cmd_rx) {
            UiCmd::Prompt(text, _) => assert_eq!(text, "/sandbox"),
            other => panic!("expected Prompt(/sandbox), got {other:?}"),
        }
    }
}

/// `/act` from idle likewise submits the prompt.
#[tokio::test]
async fn slash_act_from_idle_submits_prompt() {
    let mut chat = ChatView {
        agent: "sandbox".into(),
        ..Default::default()
    };
    let mut menu = menu_for_act();
    let (flow, mut cmd_rx, running, ..) = dispatch_popup(&mut menu, &mut chat, false, "sandbox").await;
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(running, "the switch turn starts immediately from idle");
    match drain_cmd(&mut cmd_rx) {
        UiCmd::Prompt(text, _) => assert_eq!(text, "/act"),
        other => panic!("expected Prompt(/act), got {other:?}"),
    }
}

/// `/sandbox` while running is refused by the busy gate: no command, running
/// unchanged, and a `[switch] busy` marker is pushed.
#[tokio::test]
async fn slash_sandbox_while_running_is_busy_gated() {
    let mut chat = ChatView::default();
    let mut menu = menu_for("sandbox");
    let (flow, mut cmd_rx, running, ..) = dispatch_popup(&mut menu, &mut chat, true, "act").await;
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(running, "running must stay true (turn still active)");
    assert!(
        cmd_rx.try_recv().is_err(),
        "no command should be sent while running"
    );
    assert!(
        chat.blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Marker(lines)
            if lines.iter().any(|l| l.to_string().contains("busy")))),
        "a [switch] busy marker must be pushed; blocks: {:?}",
        chat.blocks
    );
}
