//! Direct tests for `dispatch_slash_action` — the unified slash-command
//! dispatcher shared by the free-text Submit path and the `/` popup.
//!
//! `app_loop_dispatch_cmd_tests.rs` exercises `dispatch_slash_action` only
//! *indirectly* via `dispatch_command` (which delegates after a key/menu is
//! resolved into a `SlashAction`). The free-text Submit path in `app.rs`
//! feeds `command::parse` straight into `dispatch_slash_action` with no
//! `dispatch_command` in between, so these tests call the dispatcher directly
//! with a constructed `SlashAction` to cover that path end-to-end.

use super::super::*;
use crate::chat::ChatBlock;
use crate::command::SlashAction;
use opencoder_core::Config;
use opencoder_store::LibsqlStore;

/// Drain the first real UiCmd from cmd_rx, skipping the ResetCancel preamble
/// that `start_turn` always emits before the actual command.
fn drain_cmd(cmd_rx: &mut mpsc::Receiver<UiCmd>) -> UiCmd {
    let first = cmd_rx.try_recv().unwrap();
    assert!(
        matches!(first, UiCmd::ResetCancel(_)),
        "expected ResetCancel preamble, got wrong variant"
    );
    cmd_rx.try_recv().unwrap()
}

/// `/compact` dispatched directly while idle must start a compact turn:
/// `start_turn` emits `ResetCancel` then `UiCmd::Compact`, and `running`/
/// `follow` flip to true. This is the exact path the free-text Submit
/// (`app.rs`) takes when the user types `/compact` + Enter.
#[tokio::test]
async fn slash_action_compact_idle_starts_turn() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView {
        agent: "act".into(),
        ..Default::default()
    };
    let mut running = false;
    let mut follow = false;
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
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();

    let flow = dispatch_slash_action(
        SlashAction::Compact,
        &cmd_tx,
        &mut cancel,
        &mut chat,
        &mut running,
        &mut follow,
        &store,
        "test",
        &mut task_picker,
        &mut model_menu,
        &mut mcp_menu,
        &mut cache_salt_menu,
        "act",
        &mut input,
        &mut cursor_idx,
        &mut config,
        workdir,
        &mut mode_flash,
        0,
        &mut sys_tokens,
        &mut None,
        &mut None,
    )
    .await;

    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(running, "running must be true after compact dispatch");
    assert!(follow, "follow must be true after compact dispatch");
    let cmd = drain_cmd(&mut cmd_rx);
    assert!(
        matches!(cmd, UiCmd::Compact),
        "expected UiCmd::Compact, got a different UiCmd variant"
    );
}

/// `/compact` dispatched directly while a turn is already running must be a
/// guarded no-op: no `UiCmd` is sent (the running turn is untouched) and a
/// `[compact] busy` marker is pushed into the transcript.
#[tokio::test]
async fn slash_action_compact_running_pushes_busy_marker() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView {
        agent: "act".into(),
        ..Default::default()
    };
    let mut running = true;
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
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();

    let flow = dispatch_slash_action(
        SlashAction::Compact,
        &cmd_tx,
        &mut cancel,
        &mut chat,
        &mut running,
        &mut follow,
        &store,
        "test",
        &mut task_picker,
        &mut model_menu,
        &mut mcp_menu,
        &mut cache_salt_menu,
        "act",
        &mut input,
        &mut cursor_idx,
        &mut config,
        workdir,
        &mut mode_flash,
        0,
        &mut sys_tokens,
        &mut None,
        &mut None,
    )
    .await;

    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(running, "running must stay true (turn still active)");
    assert!(
        cmd_rx.try_recv().is_err(),
        "no command should be sent while a turn is running"
    );
    assert!(
        chat.blocks.iter().any(|b| matches!(
            b,
            ChatBlock::Marker(lines) if lines.iter().any(|l| l.to_string().contains("compact"))
        )),
        "a [compact] busy marker must be pushed; blocks: {:?}",
        chat.blocks
    );
}
