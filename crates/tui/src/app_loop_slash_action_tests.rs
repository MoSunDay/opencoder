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
    let mut config = Config::default();
    let workdir = std::path::Path::new(".");
    let mut mode_flash: Option<(String, u32)> = None;
    let mut sys_tokens = 0u64;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let (sidecar_tx, _sidecar_rx) = mpsc::channel::<crate::sidecar_ui::SidecarCmd>(8);
    let mut cancel = CancellationToken::new();
    let mut admit_st = crate::queue_admitter::AdmitUiState::default();
    let (admit_tx, _admit_rx) = mpsc::channel(8);
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut pending_images: Vec<(String, String)> = Vec::new();
    let mut history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;

    let flow = dispatch_slash_action(
        SlashAction::Compact,
        &cmd_tx,
        &mut cancel,
        &mut chat,
        &sidecar_tx,
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
        "act",
        &mut config,
        workdir,
        &mut mode_flash,
        0,
        &mut sys_tokens,
        &mut None,
        &mut None,
        &mut None,
        &admit_tx,
        &mut admit_st,
        &mut queue_items,
        &mut pending_images,
        &mut history,
        &mut hist_idx,
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
    let mut config = Config::default();
    let workdir = std::path::Path::new(".");
    let mut mode_flash: Option<(String, u32)> = None;
    let mut sys_tokens = 0u64;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let (sidecar_tx, _sidecar_rx) = mpsc::channel::<crate::sidecar_ui::SidecarCmd>(8);
    let mut cancel = CancellationToken::new();
    let mut admit_st = crate::queue_admitter::AdmitUiState::default();
    let (admit_tx, mut admit_rx) = mpsc::channel(8);
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut pending_images: Vec<(String, String)> = Vec::new();
    let mut history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;

    let flow = dispatch_slash_action(
        SlashAction::Compact,
        &cmd_tx,
        &mut cancel,
        &mut chat,
        &sidecar_tx,
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
        "act",
        &mut config,
        workdir,
        &mut mode_flash,
        0,
        &mut sys_tokens,
        &mut None,
        &mut None,
        &mut None,
        &admit_tx,
        &mut admit_st,
        &mut queue_items,
        &mut pending_images,
        &mut history,
        &mut hist_idx,
    )
    .await;

    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(running, "running must stay true (turn still active)");
    assert!(
        cmd_rx.try_recv().is_err(),
        "no command should be sent while a turn is running"
    );
    assert!(
        admit_rx.try_recv().is_err(),
        "compact is not a control command: nothing queues"
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

/// `/skill` (and its `/sk` alias) parses to `SlashAction::Skill` and the
/// dispatch opens the default-injection toggle modal (`SkillMenu::List`
/// built from the discovered skills merged with the config toggles).
#[tokio::test]
async fn slash_action_skill_parses_and_opens_toggle_menu() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView::default();
    let mut running = false;
    let mut follow = false;
    let mut task_picker = None;
    let mut model_menu = None;
    let mut mcp_menu: Option<crate::mcp_menu::McpMenu> = None;
    let mut cache_salt_menu = None;
    let mut skill_toggle_menu: Option<crate::skill_menu::SkillMenu> = None;
    let mut config = Config::default();
    let workdir = std::path::Path::new(".");
    let mut mode_flash: Option<(String, u32)> = None;
    let mut sys_tokens = 0u64;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let (sidecar_tx, _sidecar_rx) = mpsc::channel::<crate::sidecar_ui::SidecarCmd>(8);
    let mut cancel = CancellationToken::new();
    let mut admit_st = crate::queue_admitter::AdmitUiState::default();
    let (admit_tx, mut admit_rx) = mpsc::channel(8);
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut pending_images: Vec<(String, String)> = Vec::new();
    let mut history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;

    assert_eq!(crate::command::parse("/skill"), Some(SlashAction::Skill));
    assert_eq!(crate::command::parse("/sk"), Some(SlashAction::Skill));

    let flow = dispatch_slash_action(
        SlashAction::Skill,
        &cmd_tx,
        &mut cancel,
        &mut chat,
        &sidecar_tx,
        &mut running,
        &mut follow,
        &store,
        "test",
        &mut task_picker,
        &mut model_menu,
        &mut mcp_menu,
        &mut None,
        &mut None,
        &mut skill_toggle_menu,
        &mut None,
        &mut cache_salt_menu,
        "act",
        &mut config,
        workdir,
        &mut mode_flash,
        0,
        &mut sys_tokens,
        &mut None,
        &mut None,
        &mut None,
        &admit_tx,
        &mut admit_st,
        &mut queue_items,
        &mut pending_images,
        &mut history,
        &mut hist_idx,
    )
    .await;

    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(
        matches!(
            skill_toggle_menu,
            Some(crate::skill_menu::SkillMenu::List(_))
        ),
        "the /skill toggle modal must open"
    );
    assert!(
        cmd_rx.try_recv().is_err(),
        "opening the modal must not send a UiCmd"
    );
    assert!(admit_rx.try_recv().is_err(), "menu dispatch must not queue");
}

/// `/ap` parses to `SlashAction::Ap` and the dispatch opens the tri-state
/// mode-picker modal (`ApMenu`) with the cursor on the config's current
/// mode. No `UiCmd` is sent until the user confirms a mode in the modal.
#[tokio::test]
async fn slash_action_ap_parses_and_opens_mode_menu() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView::default();
    let mut running = false;
    let mut follow = false;
    let mut task_picker = None;
    let mut model_menu = None;
    let mut mcp_menu: Option<crate::mcp_menu::McpMenu> = None;
    let mut cache_salt_menu = None;
    let mut ap_menu: Option<crate::ap_menu::ApMenu> = None;
    let mut config = Config::default();
    config.autopilot.mode = opencoder_core::ApMode::Review;
    let workdir = std::path::Path::new(".");
    let mut mode_flash: Option<(String, u32)> = None;
    let mut sys_tokens = 0u64;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let (sidecar_tx, _sidecar_rx) = mpsc::channel::<crate::sidecar_ui::SidecarCmd>(8);
    let mut cancel = CancellationToken::new();
    let mut admit_st = crate::queue_admitter::AdmitUiState::default();
    let (admit_tx, mut admit_rx) = mpsc::channel(8);
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut pending_images: Vec<(String, String)> = Vec::new();
    let mut history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;

    assert_eq!(crate::command::parse("/ap"), Some(SlashAction::Ap));

    let flow = dispatch_slash_action(
        SlashAction::Ap,
        &cmd_tx,
        &mut cancel,
        &mut chat,
        &sidecar_tx,
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
        &mut ap_menu,
        &mut cache_salt_menu,
        "act",
        &mut config,
        workdir,
        &mut mode_flash,
        0,
        &mut sys_tokens,
        &mut None,
        &mut None,
        &mut None,
        &admit_tx,
        &mut admit_st,
        &mut queue_items,
        &mut pending_images,
        &mut history,
        &mut hist_idx,
    )
    .await;

    assert!(matches!(flow, LoopFlow::Proceed));
    let menu = ap_menu.expect("the /ap mode-picker modal must open");
    assert_eq!(
        menu.selected, 2,
        "cursor highlights the config's current (review) mode"
    );
    assert_eq!(menu.current, opencoder_core::ApMode::Review);
    assert!(
        cmd_rx.try_recv().is_err(),
        "opening the modal must not send a UiCmd"
    );
    assert!(admit_rx.try_recv().is_err(), "menu dispatch must not queue");
}

/// `/sidecar` dispatched while idle opens the (fresh) panel: a placeholder
/// block is pushed and focused, `follow` flips on for the body swap, and a
/// `SidecarCmd::Reset` reaches the actor (entry destroys the previous
/// conversation). The turn state is untouched.
#[tokio::test]
async fn slash_action_sidecar_idle_opens_fresh_panel() {
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
    let mut config = Config::default();
    let workdir = std::path::Path::new(".");
    let mut mode_flash: Option<(String, u32)> = None;
    let mut sys_tokens = 0u64;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let (sidecar_tx, mut sidecar_rx) = mpsc::channel::<crate::sidecar_ui::SidecarCmd>(8);
    let mut cancel = CancellationToken::new();
    let mut admit_st = crate::queue_admitter::AdmitUiState::default();
    let (admit_tx, _admit_rx) = mpsc::channel(8);
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut pending_images: Vec<(String, String)> = Vec::new();
    let mut history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;

    let flow = dispatch_slash_action(
        SlashAction::Sidecar,
        &cmd_tx,
        &mut cancel,
        &mut chat,
        &sidecar_tx,
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
        "act",
        &mut config,
        workdir,
        &mut mode_flash,
        0,
        &mut sys_tokens,
        &mut None,
        &mut None,
        &mut None,
        &admit_tx,
        &mut admit_st,
        &mut queue_items,
        &mut pending_images,
        &mut history,
        &mut hist_idx,
    )
    .await;

    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(!running, "opening the panel must not start a turn");
    assert!(chat.sidecar_focus, "panel is focused");
    assert_eq!(
        chat.blocks
            .iter()
            .filter(|b| matches!(b, ChatBlock::Sidecar { .. }))
            .count(),
        1,
        "exactly the fresh placeholder block"
    );
    assert!(
        matches!(
            sidecar_rx.try_recv(),
            Ok(crate::sidecar_ui::SidecarCmd::Reset)
        ),
        "entry must send Reset to the actor"
    );
    assert!(cmd_rx.try_recv().is_err(), "no UiCmd is sent");
    assert!(
        matches!(&mode_flash, Some((msg, _)) if msg == crate::sidecar_ui::SIDECAR_ENTER_FLASH),
        "composer carries the enter flash, got {mode_flash:?}"
    );
    assert!(follow, "body follows the panel");
}

/// `/sidecar` dispatched MID-TURN still opens the panel: the sidecar bypasses
/// the parent's steer/queue paths entirely, so the running gate does not
/// apply and the running turn is untouched (no ResetCancel, no UiCmd).
#[tokio::test]
async fn slash_action_sidecar_running_still_opens_panel() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView {
        agent: "act".into(),
        ..Default::default()
    };
    let mut running = true;
    let mut follow = false;
    let mut task_picker = None;
    let mut model_menu = None;
    let mut mcp_menu: Option<crate::mcp_menu::McpMenu> = None;
    let mut cache_salt_menu = None;
    let mut config = Config::default();
    let workdir = std::path::Path::new(".");
    let mut mode_flash: Option<(String, u32)> = None;
    let mut sys_tokens = 0u64;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let (sidecar_tx, mut sidecar_rx) = mpsc::channel::<crate::sidecar_ui::SidecarCmd>(8);
    let mut cancel = CancellationToken::new();
    let mut admit_st = crate::queue_admitter::AdmitUiState::default();
    let (admit_tx, _admit_rx) = mpsc::channel(8);
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut pending_images: Vec<(String, String)> = Vec::new();
    let mut history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;

    let flow = dispatch_slash_action(
        SlashAction::Sidecar,
        &cmd_tx,
        &mut cancel,
        &mut chat,
        &sidecar_tx,
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
        "act",
        &mut config,
        workdir,
        &mut mode_flash,
        0,
        &mut sys_tokens,
        &mut None,
        &mut None,
        &mut None,
        &admit_tx,
        &mut admit_st,
        &mut queue_items,
        &mut pending_images,
        &mut history,
        &mut hist_idx,
    )
    .await;

    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(running, "the parent turn keeps running");
    assert!(chat.sidecar_focus, "panel opened despite the running turn");
    assert!(matches!(
        sidecar_rx.try_recv(),
        Ok(crate::sidecar_ui::SidecarCmd::Reset)
    ));
    assert!(
        cmd_rx.try_recv().is_err(),
        "no parent UiCmd is sent (bypass path)"
    );
}
