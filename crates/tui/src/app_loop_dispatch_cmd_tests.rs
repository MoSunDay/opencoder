//! dispatch_command tests for the /act and /act_clear_context plan->act
//! handoff behavior.
//!
//! Guards two regressions:
//! 1. From plan mode with a submitted plan (idle), both /act and
//!    /act_clear_context must route through SwitchAndStart (plan->act handoff),
//!    preserving the plan and starting execution -- same as Shift+Tab.
//! 2. From act mode (or plan without a plan), they must still dispatch the
//!    control-command prompt (the original wipe/toggle behavior).

use super::super::*;
use crate::chat::ChatBlock;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::Config;
use opencoder_store::LibsqlStore;

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
    cm.paste("act"); // rows: [/compact, /act, /act_clear_context]
    cm.move_down(); // select /act (2nd match)
    Some(cm)
}

fn enter_key() -> KeyEvent {
    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
}

fn tab_key() -> KeyEvent {
    KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)
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

/// `/act` from plan mode with a submitted plan (idle) must route through
/// SwitchAndStart (handoff), not Prompt("/act").
#[tokio::test]
async fn slash_act_from_plan_with_plan_routes_handoff() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView {
        agent: "plan".into(),
        plan_submitted: true,
        ..Default::default()
    };
    let mut running = false;
    let mut follow = false;
    let mut task_picker = None;
    let mut model_menu = None;
    let mut cache_salt_menu = None;
    let mut input = String::new();
    let mut cursor_idx = 0usize;
    let mut config = Config::default();
    let workdir = std::path::Path::new(".");
    let mut mode_flash: Option<(String, u32)> = None;
    let mut sys_tokens = 0u64;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut command_menu = menu_for_act();

    let flow = dispatch_command(
        &mut command_menu,
        enter_key(),
        &cmd_tx,
        &mut cancel,
        &mut chat,
        &mut running,
        &mut follow,
        &store,
        "test",
        &mut task_picker,
        &mut model_menu,
        &mut cache_salt_menu,
        &mut None,
        "plan",
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
    assert!(running, "running must be true after dispatch");
    match drain_cmd(&mut cmd_rx) {
        UiCmd::SwitchAndStart(ref n, _) => assert_eq!(n, "act"),
        _other => panic!("expected SwitchAndStart, got unexpected variant"),
    }
    assert!(mode_flash.is_some(), "mode_flash must be set on handoff");
}

/// `/act_clear_context` from plan mode with a submitted plan (idle) must also
/// route through SwitchAndStart (handoff preserves the plan).
#[tokio::test]
async fn slash_clear_context_from_plan_with_plan_routes_handoff() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView {
        agent: "plan".into(),
        plan_submitted: true,
        ..Default::default()
    };
    let mut running = false;
    let mut follow = false;
    let mut task_picker = None;
    let mut model_menu = None;
    let mut cache_salt_menu = None;
    let mut input = String::new();
    let mut cursor_idx = 0usize;
    let mut config = Config::default();
    let workdir = std::path::Path::new(".");
    let mut mode_flash: Option<(String, u32)> = None;
    let mut sys_tokens = 0u64;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut command_menu = menu_for("act_clear");

    let flow = dispatch_command(
        &mut command_menu,
        enter_key(),
        &cmd_tx,
        &mut cancel,
        &mut chat,
        &mut running,
        &mut follow,
        &store,
        "test",
        &mut task_picker,
        &mut model_menu,
        &mut cache_salt_menu,
        &mut None,
        "plan",
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
    assert!(running, "running must be true after dispatch");
    match drain_cmd(&mut cmd_rx) {
        UiCmd::SwitchAndStart(ref n, _) => assert_eq!(n, "act"),
        _other => panic!("expected SwitchAndStart, got unexpected variant"),
    }
    assert!(mode_flash.is_some(), "mode_flash must be set on handoff");
}

/// `/act_clear_context` from act mode (no plan) must still dispatch the
/// Prompt("/act_clear_context") control command (wipe behavior).
#[tokio::test]
async fn slash_clear_context_from_act_mode_dispatches_prompt() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView {
        agent: "act".into(),
        plan_submitted: false,
        ..Default::default()
    };
    let mut running = false;
    let mut follow = false;
    let mut task_picker = None;
    let mut model_menu = None;
    let mut cache_salt_menu = None;
    let mut input = String::new();
    let mut cursor_idx = 0usize;
    let mut config = Config::default();
    let workdir = std::path::Path::new(".");
    let mut mode_flash: Option<(String, u32)> = None;
    let mut sys_tokens = 0u64;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut command_menu = menu_for("act_clear");

    let flow = dispatch_command(
        &mut command_menu,
        enter_key(),
        &cmd_tx,
        &mut cancel,
        &mut chat,
        &mut running,
        &mut follow,
        &store,
        "test",
        &mut task_picker,
        &mut model_menu,
        &mut cache_salt_menu,
        &mut None,
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
    assert!(running, "running must be true after dispatch");
    match drain_cmd(&mut cmd_rx) {
        UiCmd::Prompt(ref p, _) => assert_eq!(p, "/act_clear_context"),
        _other => panic!("expected Prompt(\"/act_clear_context\"), got unexpected variant"),
    }
    assert!(
        mode_flash.is_none(),
        "mode_flash must NOT be set for plain wipe"
    );
}

/// `/act` from act mode must still dispatch Prompt("/act") (toggle, no-op).
#[tokio::test]
async fn slash_act_from_act_mode_dispatches_prompt() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView {
        agent: "act".into(),
        plan_submitted: false,
        ..Default::default()
    };
    let mut running = false;
    let mut follow = false;
    let mut task_picker = None;
    let mut model_menu = None;
    let mut cache_salt_menu = None;
    let mut input = String::new();
    let mut cursor_idx = 0usize;
    let mut config = Config::default();
    let workdir = std::path::Path::new(".");
    let mut mode_flash: Option<(String, u32)> = None;
    let mut sys_tokens = 0u64;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut command_menu = menu_for_act();

    let flow = dispatch_command(
        &mut command_menu,
        enter_key(),
        &cmd_tx,
        &mut cancel,
        &mut chat,
        &mut running,
        &mut follow,
        &store,
        "test",
        &mut task_picker,
        &mut model_menu,
        &mut cache_salt_menu,
        &mut None,
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
    assert!(running, "running must be true after dispatch");
    match drain_cmd(&mut cmd_rx) {
        UiCmd::Prompt(ref p, _) => assert_eq!(p, "/act"),
        _other => panic!("expected Prompt(\"/act\"), got unexpected variant"),
    }
    assert!(
        mode_flash.is_none(),
        "mode_flash must NOT be set for plain toggle"
    );
}

/// `/act` from plan mode WITHOUT a submitted plan must dispatch Prompt("/act")
/// (plain toggle, no handoff).
#[tokio::test]
async fn slash_act_from_plan_without_plan_dispatches_prompt() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView {
        agent: "plan".into(),
        plan_submitted: false,
        ..Default::default()
    };
    let mut running = false;
    let mut follow = false;
    let mut task_picker = None;
    let mut model_menu = None;
    let mut cache_salt_menu = None;
    let mut input = String::new();
    let mut cursor_idx = 0usize;
    let mut config = Config::default();
    let workdir = std::path::Path::new(".");
    let mut mode_flash: Option<(String, u32)> = None;
    let mut sys_tokens = 0u64;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut command_menu = menu_for_act();

    let flow = dispatch_command(
        &mut command_menu,
        enter_key(),
        &cmd_tx,
        &mut cancel,
        &mut chat,
        &mut running,
        &mut follow,
        &store,
        "test",
        &mut task_picker,
        &mut model_menu,
        &mut cache_salt_menu,
        &mut None,
        "plan",
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
    assert!(running, "running must be true after dispatch");
    match drain_cmd(&mut cmd_rx) {
        UiCmd::Prompt(ref p, _) => assert_eq!(p, "/act"),
        _other => panic!("expected Prompt(\"/act\"), got unexpected variant"),
    }
}

/// Tab on a command in the popup fills the composer input with the command
/// name plus a trailing space (so the user can type args or press Enter
/// immediately). The popup closes and the composer is ready. No turn starts.
#[tokio::test]
async fn tab_fill_input_adds_trailing_space() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView::default();
    let mut running = false;
    let mut follow = false;
    let mut task_picker = None;
    let mut model_menu = None;
    let mut cache_salt_menu = None;
    let mut input = String::new();
    let mut cursor_idx = 0usize;
    let mut config = Config::default();
    let workdir = std::path::Path::new(".");
    let mut mode_flash: Option<(String, u32)> = None;
    let mut sys_tokens = 0u64;
    let (cmd_tx, _cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut command_menu = menu_for("plan");

    let flow = dispatch_command(
        &mut command_menu,
        tab_key(),
        &cmd_tx,
        &mut cancel,
        &mut chat,
        &mut running,
        &mut follow,
        &store,
        "test",
        &mut task_picker,
        &mut model_menu,
        &mut cache_salt_menu,
        &mut None,
        "plan",
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

    assert!(
        input.starts_with('/'),
        "input must start with /, got {input:?}"
    );
    assert!(
        input.ends_with(' '),
        "input must end with a trailing space, got {input:?}"
    );
    assert_eq!(input, "/plan ");
    assert_eq!(cursor_idx, input.len(), "cursor must sit after the space");
    assert!(!running, "FillInput must not start a turn");
    assert!(command_menu.is_none(), "popup must close after Tab-fill");
    assert!(
        matches!(flow, LoopFlow::Redraw),
        "FillInput must request a redraw"
    );
}

/// The trailing-space UX is uniform across command categories: a local
/// command (`/ps`) also gets the trailing space, so Enter still dispatches
/// correctly (parse trims) while args can be appended.
#[tokio::test]
async fn tab_fill_local_command_adds_trailing_space() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView::default();
    let mut running = false;
    let mut follow = false;
    let mut task_picker = None;
    let mut model_menu = None;
    let mut cache_salt_menu = None;
    let mut input = String::new();
    let mut cursor_idx = 0usize;
    let mut config = Config::default();
    let workdir = std::path::Path::new(".");
    let mut mode_flash: Option<(String, u32)> = None;
    let mut sys_tokens = 0u64;
    let (cmd_tx, _cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut command_menu = menu_for("ps");

    let flow = dispatch_command(
        &mut command_menu,
        tab_key(),
        &cmd_tx,
        &mut cancel,
        &mut chat,
        &mut running,
        &mut follow,
        &store,
        "test",
        &mut task_picker,
        &mut model_menu,
        &mut cache_salt_menu,
        &mut None,
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

    assert_eq!(input, "/ps ");
    assert!(input.ends_with(' '));
    assert_eq!(cursor_idx, input.len());
    assert!(command_menu.is_none());
    assert!(matches!(flow, LoopFlow::Redraw));
}

// ----- running-gate tests (/act /plan /act_clear_context while running) -----

/// `/plan` while a turn is running must NOT start a new turn: a marker is
/// pushed, `running` stays true, and no UiCmd (in particular no Prompt) leaks
/// to the worker. Mirrors `/compact`'s SkipRunning feedback.
#[tokio::test]
async fn slash_plan_while_running_is_noop() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView {
        agent: "act".into(),
        ..Default::default()
    };
    let mut running = true;
    let mut follow = true;
    let mut task_picker = None;
    let mut model_menu = None;
    let mut cache_salt_menu = None;
    let mut input = String::new();
    let mut cursor_idx = 0usize;
    let mut config = Config::default();
    let workdir = std::path::Path::new(".");
    let mut mode_flash: Option<(String, u32)> = None;
    let mut sys_tokens = 0u64;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut command_menu = menu_for("plan");

    let flow = dispatch_command(
        &mut command_menu,
        enter_key(),
        &cmd_tx,
        &mut cancel,
        &mut chat,
        &mut running,
        &mut follow,
        &store,
        "test",
        &mut task_picker,
        &mut model_menu,
        &mut cache_salt_menu,
        &mut None,
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
        "no command should be sent while running"
    );
    // The busy marker must be pushed into the transcript.
    assert!(
        chat.blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Marker(lines)
            if lines.iter().any(|l| l.to_string().contains("busy")))),
        "a [switch] busy marker must be pushed; blocks: {:?}",
        chat.blocks
    );
}

/// `/act` while a turn is running is a no-op — even from plan mode with a
/// submitted plan, which would otherwise route through SwitchAndStart and
/// start a NEW turn mid-flight.
#[tokio::test]
async fn slash_act_while_running_is_noop() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView {
        agent: "plan".into(),
        plan_submitted: true,
        ..Default::default()
    };
    let mut running = true;
    let mut follow = true;
    let mut task_picker = None;
    let mut model_menu = None;
    let mut cache_salt_menu = None;
    let mut input = "do it".to_string();
    let mut cursor_idx = 5usize;
    let mut config = Config::default();
    let workdir = std::path::Path::new(".");
    let mut mode_flash: Option<(String, u32)> = None;
    let mut sys_tokens = 0u64;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut command_menu = menu_for_act();

    let flow = dispatch_command(
        &mut command_menu,
        enter_key(),
        &cmd_tx,
        &mut cancel,
        &mut chat,
        &mut running,
        &mut follow,
        &store,
        "test",
        &mut task_picker,
        &mut model_menu,
        &mut cache_salt_menu,
        &mut None,
        "plan",
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
        "no command should be sent while running"
    );
    assert_eq!(input, "do it", "input must be untouched on the noop");
    assert!(
        chat.blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Marker(lines)
            if lines.iter().any(|l| l.to_string().contains("busy")))),
        "a [switch] busy marker must be pushed; blocks: {:?}",
        chat.blocks
    );
}

/// `/act_clear_context` while a turn is running is a no-op (same gate).
#[tokio::test]
async fn slash_clear_context_while_running_is_noop() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView {
        agent: "plan".into(),
        plan_submitted: true,
        ..Default::default()
    };
    let mut running = true;
    let mut follow = true;
    let mut task_picker = None;
    let mut model_menu = None;
    let mut cache_salt_menu = None;
    let mut input = String::new();
    let mut cursor_idx = 0usize;
    let mut config = Config::default();
    let workdir = std::path::Path::new(".");
    let mut mode_flash: Option<(String, u32)> = None;
    let mut sys_tokens = 0u64;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut command_menu = menu_for("act_clear");

    let flow = dispatch_command(
        &mut command_menu,
        enter_key(),
        &cmd_tx,
        &mut cancel,
        &mut chat,
        &mut running,
        &mut follow,
        &store,
        "test",
        &mut task_picker,
        &mut model_menu,
        &mut cache_salt_menu,
        &mut None,
        "plan",
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

/// `/act` while a subagent is live is a no-op — running=false but
/// subagents_running>0 (the dropped-SubagentEnd window: Done/TurnDone clear
/// `running` and zero the counter in `apply`, but a forwarded SubagentEnd can
/// be dropped under channel saturation). The gate must still refuse a mode
/// switch while a tracked subagent exists. Uses plan_submitted=true so it
/// WOULD route to SwitchAndStart (start a NEW turn) if not gated.
#[tokio::test]
async fn slash_act_while_subagent_running_is_noop() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView {
        agent: "plan".into(),
        plan_submitted: true,
        subagents_running: 1,
        ..Default::default()
    };
    let mut running = false;
    let mut follow = true;
    let mut task_picker = None;
    let mut model_menu = None;
    let mut cache_salt_menu = None;
    let mut input = "do it".to_string();
    let mut cursor_idx = 5usize;
    let mut config = Config::default();
    let workdir = std::path::Path::new(".");
    let mut mode_flash: Option<(String, u32)> = None;
    let mut sys_tokens = 0u64;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut command_menu = menu_for_act();

    let flow = dispatch_command(
        &mut command_menu,
        enter_key(),
        &cmd_tx,
        &mut cancel,
        &mut chat,
        &mut running,
        &mut follow,
        &store,
        "test",
        &mut task_picker,
        &mut model_menu,
        &mut cache_salt_menu,
        &mut None,
        "plan",
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
    assert!(
        !running,
        "running must stay false (no turn started while a subagent is live)"
    );
    assert!(
        cmd_rx.try_recv().is_err(),
        "no command should be sent while a subagent is live"
    );
    assert_eq!(input, "do it", "input must be untouched on the noop");
    assert!(
        chat.blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Marker(lines)
            if lines.iter().any(|l| l.to_string().contains("busy")))),
        "a [switch] busy marker must be pushed; blocks: {:?}",
        chat.blocks
    );
}
