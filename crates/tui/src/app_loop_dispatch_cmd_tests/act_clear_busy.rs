//! Manual-confirm regression for the SUBAGENT-DRAIN busy window: while the
//! parent already reports idle (`running == false`) its subagent batch may
//! still be live (`chat.subagents_running > 0`). The second Shift+Tab must
//! still interrupt (cancel) and fire the `/act_clear_context` turn right
//! away — gating the interrupt on `running` alone made the manual confirm
//! queue the fold behind the still-draining turn, so act mode did not
//! execute immediately (the double-Shift+Tab "looks idle" bug).

use super::*;

#[tokio::test]
async fn manual_confirm_during_subagent_drain_fires_now() {
    let mut chat = ChatView {
        agent: "act".into(),
        subagents_running: 3,
        ..Default::default()
    };
    let mut clear_confirm = None;
    let mut mode_flash: Option<(String, u32)> = None;
    let mut input = String::new();
    let mut cursor_idx = 0;
    let mut undo_state = crate::undo::UndoState::default();
    let mut running = false; // parent idle — subagents still draining
    let mut follow = false;
    let mut sys_tokens = 0u64;
    let mut history: Vec<String> = Vec::new();
    let mut hist_idx = None;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut pending_images: Vec<(String, String)> = Vec::new();
    let mut admit_st = crate::queue_admitter::AdmitUiState::default();
    let (admit_tx, mut admit_rx) = mpsc::channel(8);
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut turn_gen = 0u64;
    let mut child_runtime = crate::worker::ChildRuntimeHandles {
        cancels: Default::default(),
        turn_cancels: Default::default(),
        steer_gates: Default::default(),
    };
    let mut cancelled = false;

    crate::clear_confirm::engage(
        &mut clear_confirm,
        &mut chat,
        &mut mode_flash,
        0,
        Some("finish the summary".into()),
        None,
    );

    let backtab = KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE);
    crate::app::app_loop::handle_confirm_key(
        &mut clear_confirm,
        backtab,
        &mut input,
        &mut cursor_idx,
        &mut undo_state,
        &mut chat,
        &cmd_tx,
        &mut cancel,
        &mut turn_gen,
        &mut running,
        &mut follow,
        &mut sys_tokens,
        &mut mode_flash,
        0,
        std::path::Path::new("."),
        &admit_tx,
        &mut admit_st,
        &mut queue_items,
        &mut pending_images,
        "test",
        &mut history,
        &mut hist_idx,
        &mut child_runtime,
        &mut cancelled,
    )
    .await;

    assert!(clear_confirm.is_none(), "the manual confirm consumes the arm");
    assert!(
        cancelled,
        "live subagents count as busy: the drain is hard-cancelled"
    );
    assert!(
        chat.blocks.iter().any(|b| matches!(b, ChatBlock::Marker(lines)
            if lines.iter().any(|l| l.to_string().contains("[interrupted] stopping")))),
        "the interruption must be visible; blocks: {:?}",
        chat.blocks
    );
    match drain_cmd(&mut cmd_rx) {
        UiCmd::Prompt(text, _) => assert_eq!(
            text, "/act_clear_context finish the summary",
            "act fires immediately even while the parent reports idle"
        ),
        other => panic!("expected immediate Prompt, got {other:?}"),
    }
    assert!(running, "the clear turn is the new running turn");
    assert!(
        admit_rx.try_recv().is_err(),
        "manual confirm never queues — it interrupts instead"
    );
}
