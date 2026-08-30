//! `/act_clear_context` popup dispatch: arms the countdown guard (misop
//! protection) instead of firing outright — from idle AND while running.
//! Firing submits the canonical control-command prompt when idle and queues
//! it verbatim when running. (An act session emits only `TranscriptReset` +
//! `Done`; a sandbox session additionally converges to act with one extra
//! `AgentSwitch`.)
use super::*;

/// Popup dispatch of the clear-context command arms the guard: no UiCmd is
/// sent, the countdown chip is up, and the transcript gets exactly one
/// countdown marker (no seed echo, no key affordances).
#[tokio::test]
async fn slash_clear_context_arms_countdown_guard() {
    let mut chat = ChatView {
        agent: "act".into(),
        ..Default::default()
    };
    let mut menu = menu_for("clear");
    let (flow, mut cmd_rx, running, confirm, flash, markers) =
        dispatch_popup(&mut menu, &mut chat, false, "act").await;
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(!running, "arming must not start a turn");
    assert!(
        cmd_rx.try_recv().is_err(),
        "arming must not send any command"
    );
    let cc = confirm.expect("the guard must be armed");
    assert_eq!(cc.rest, None);
    let (chip, _) = flash.expect("countdown chip must be raised");
    assert!(chip.contains("之后仅保留计划并执行"), "chip: {chip}");
    assert_eq!(markers.len(), 1, "exactly one countdown marker: {markers:?}");
    assert!(
        markers[0].contains("5s 之后仅保留计划并执行"),
        "countdown marker: {:?}",
        markers[0]
    );
}

/// While running the guard arms too — firing later queues at the idle
/// boundary instead of hard-refusing, matching the Shift+Tab semantics.
#[tokio::test]
async fn slash_clear_context_while_running_arms_guard() {
    let mut chat = ChatView {
        agent: "act".into(),
        ..Default::default()
    };
    let mut menu = menu_for("clear");
    let (flow, mut cmd_rx, running, confirm, _, _) =
        dispatch_popup(&mut menu, &mut chat, true, "act").await;
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(running, "running must stay true (turn still active)");
    assert!(
        cmd_rx.try_recv().is_err(),
        "no command should be sent while running"
    );
    assert!(confirm.is_some(), "the guard must be armed even while running");
}

/// Firing the guard from idle submits the canonical control-command prompt
/// and starts the turn (mirrors the mode-switch Run arm).
#[tokio::test]
async fn fired_guard_submits_canonical_prompt_when_idle() {
    let mut chat = ChatView {
        agent: "act".into(),
        ..Default::default()
    };
    let mut running = false;
    let mut follow = false;
    let mut sys_tokens = 0u64;
    let mut mode_flash: Option<(String, u32)> = None;
    let mut history: Vec<String> = Vec::new();
    let mut hist_idx = None;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut pending_images: Vec<(String, String)> = Vec::new();
    let mut admit_st = crate::queue_admitter::AdmitUiState::default();
    let (admit_tx, mut admit_rx) = mpsc::channel(8);
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let flow = crate::app::app_loop::fire_clear_confirm(
        crate::clear_confirm::arm(Some("finish the summary".into()), None),
        &cmd_tx, &mut cancel, &mut running, &mut follow, &mut chat,
        &mut sys_tokens, &mut mode_flash, 0, std::path::Path::new("."),
        &admit_tx, &mut admit_st, &mut queue_items, &mut pending_images,
        "test", &mut history, &mut hist_idx,
    )
    .await;
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(running, "the clear turn starts from idle");
    match drain_cmd(&mut cmd_rx) {
        UiCmd::Prompt(text, _) => {
            assert_eq!(text, "/act_clear_context finish the summary")
        }
        other => panic!("expected compound Prompt, got {other:?}"),
    }
    assert!(admit_rx.try_recv().is_err(), "idle fire must not queue");
}

/// Firing the guard while running queues the compound command verbatim —
/// the runner applies it at the idle boundary.
#[tokio::test]
async fn fired_guard_queues_compound_when_running() {
    let mut chat = ChatView {
        agent: "act".into(),
        ..Default::default()
    };
    let mut running = true;
    let mut follow = true;
    let mut sys_tokens = 42u64;
    let mut mode_flash: Option<(String, u32)> = None;
    let mut history: Vec<String> = Vec::new();
    let mut hist_idx = None;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut pending_images: Vec<(String, String)> = Vec::new();
    let mut admit_st = crate::queue_admitter::AdmitUiState::default();
    let (admit_tx, mut admit_rx) = mpsc::channel(8);
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let flow = crate::app::app_loop::fire_clear_confirm(
        crate::clear_confirm::arm(Some("then run checks".into()), None),
        &cmd_tx, &mut cancel, &mut running, &mut follow, &mut chat,
        &mut sys_tokens, &mut mode_flash, 0, std::path::Path::new("."),
        &admit_tx, &mut admit_st, &mut queue_items, &mut pending_images,
        "test", &mut history, &mut hist_idx,
    )
    .await;
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(running, "the in-flight turn stays untouched");
    assert!(
        cmd_rx.try_recv().is_err(),
        "running fire must not start a new turn"
    );
    let req = admit_rx.try_recv().expect("running fire must queue");
    assert_eq!(req.display, "/act_clear_context then run checks");
}

/// Shift+Tab and typing the command are ONE path: every spelling parses to
/// [`SlashAction::ClearContext`], the canonical string round-trips through
/// `control_cmd_string`, and the compound tail survives `head_rest`.
#[test]
fn backtab_and_typed_clear_context_are_one_path() {
    use crate::clear_confirm::{head_rest, CLEAR_CONTEXT_CMD};
    use crate::command::{control_cmd_string, parse, SlashAction};

    // Every spelling the UI can produce parses to the same action.
    assert_eq!(parse(CLEAR_CONTEXT_CMD), Some(SlashAction::ClearContext));
    assert_eq!(parse("/clear_context"), Some(SlashAction::ClearContext));
    assert_eq!(parse("/act_clear_context"), Some(SlashAction::ClearContext));

    // The canonical submitted text is the runner's ClearContext control
    // command and round-trips.
    assert_eq!(
        control_cmd_string(&SlashAction::ClearContext),
        Some(CLEAR_CONTEXT_CMD)
    );

    // A Shift+Tab with a draft forwards it as the compound rest of the SAME
    // command — equivalent to typing "/act_clear_context <draft>".
    let typed = format!("{CLEAR_CONTEXT_CMD} finish the summary");
    let (cmd, rest) =
        opencoder_session::split_control_prefix(&typed).expect("compound must parse");
    assert!(matches!(cmd, opencoder_session::ControlCmd::ClearContext));
    assert_eq!(rest, Some("finish the summary".into()));
    assert_eq!(
        head_rest("/clear_context legacy tail"),
        Some(Some("legacy tail".into()))
    );
}

/// Esc on the armed guard must drop the countdown chip itself: idle freezes
/// `anim_tick` once the guard is gone, so a leftover mode-flash would be
/// pinned on screen forever — the cancel path has to clear it explicitly.
#[tokio::test]
async fn esc_cancel_drops_countdown_chip() {
    let mut chat = ChatView {
        agent: "act".into(),
        ..Default::default()
    };
    let mut clear_confirm = None;
    let mut mode_flash: Option<(String, u32)> = None;
    let mut input = String::from("draft text");
    let mut cursor_idx = input.len();
    let mut undo_state = crate::undo::UndoState::default();
    let mut running = false;
    let mut follow = false;
    let mut sys_tokens = 0u64;
    let mut history: Vec<String> = Vec::new();
    let mut hist_idx = None;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut pending_images: Vec<(String, String)> = Vec::new();
    let mut admit_st = crate::queue_admitter::AdmitUiState::default();
    let (admit_tx, _admit_rx) = mpsc::channel(8);
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();

    crate::clear_confirm::engage(
        &mut clear_confirm,
        &mut chat,
        &mut mode_flash,
        0,
        None,
        Some("draft text".into()),
    );
    assert!(clear_confirm.is_some(), "the guard must be armed");
    assert!(mode_flash.is_some(), "countdown chip must be raised");

    let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
    crate::app::app_loop::handle_confirm_key(
        &mut clear_confirm, esc, &mut input, &mut cursor_idx,
        &mut undo_state, &mut chat, &cmd_tx, &mut cancel,
        &mut running, &mut follow, &mut sys_tokens, &mut mode_flash,
        0, std::path::Path::new("."), &admit_tx, &mut admit_st,
        &mut queue_items, &mut pending_images, "test", &mut history,
        &mut hist_idx,
    )
    .await;

    assert!(mode_flash.is_none(), "the countdown chip must be gone");
    assert!(clear_confirm.is_none(), "the guard must be torn down");
    assert!(
        chat.blocks.iter().any(|b| matches!(b, ChatBlock::Marker(lines)
        if lines.iter().any(|l| l.to_string().contains("已取消（回撤）")))),
        "a cancel (回撤) marker must be pushed; blocks: {:?}",
        chat.blocks
    );
    assert_eq!(input, "draft text", "the draft must be restored");
    assert_eq!(cursor_idx, input.len(), "cursor sits at the draft end");
    assert!(
        cmd_rx.try_recv().is_err(),
        "cancel must not send any command"
    );
}
