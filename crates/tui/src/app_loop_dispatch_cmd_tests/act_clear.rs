//! `/act_clear_context` running-gate tests, split out of
//! `app_loop_dispatch_cmd_tests/mod.rs` to keep both files under the
//! 800-line iteration cap. Shared helpers (`menu_for`, `enter_key`,
//! `drain_cmd`) live in the parent module.

use super::*;
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
        &mut mcp_menu,
        &mut None,
        &mut None,
        &mut None,
        &mut None,
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
