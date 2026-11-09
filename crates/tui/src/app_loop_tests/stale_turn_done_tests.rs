//! Turn-generation guard regression tests for `fold_ui_events`'s TurnDone
//! arm. The worker tags every `TurnDone` with the generation of its most
//! recent `ResetCancel`; the loop drops completions whose generation no
//! longer matches (`cancel_running_turn` / `start_turn` bump the counter).
//! Before the guard, a stale completion from a cancelled turn flipped
//! `running` off and reassigned `chat.agent` while the superseding turn
//! (e.g. the double-Shift+Tab `/act_clear_context` fire) was still queued —
//! the UI looked idle and the new turn seemed not to execute.

use super::*;

use opencoder_store::{Delivery, SessionInput, SessionMeta, Store, TursoStore};

/// Arm `drain_pending` like a cancelled-but-draining turn: session with one
/// stranded queued row, `running = true`, then a `Done` folded in.
#[allow(clippy::too_many_arguments)]
async fn armed_drain(
    sid: &str,
    store: &Arc<dyn Store>,
    chat: &mut ChatView,
    running: &mut bool,
    drain_pending: &mut bool,
    cancelled: &mut bool,
    turn_gen: &mut u64,
    cmd_tx: &mpsc::Sender<UiCmd>,
    cancel: &mut CancellationToken,
    evt_rx: &mut mpsc::Receiver<UiEvent>,
) {
    store
        .admit_input(&SessionInput {
            seq: None,
            id: "q-1".into(),
            session_id: sid.into(),
            delivery: Delivery::Queue,
            prompt: "stranded".into(),
            images: vec![],
            admitted_seq: 0,
            promoted_seq: None,
            display_text: None,
        })
        .await
        .unwrap();
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut skip_next_render = false;
    let mut follow = true;
    let mut notepad: Option<crate::notepad::NotepadView> = None;
    let _ = fold_ui_events(
        Some(UiEvent::Session(SessionEvent::Done)),
        chat,
        store,
        sid,
        &mut queue_items,
        &mut false,
        &mut crate::queue_admitter::AdmitUiState::default(),
        running,
        cancelled,
        drain_pending,
        &mut skip_next_render,
        &mut follow,
        cmd_tx,
        cancel,
        turn_gen,
        evt_rx,
        &mut notepad,
        &mut None,
        &opencoder_session::QuestionHub::new(),
    )
    .await;
    assert!(*drain_pending, "fixture: Done with a stranded row arms the drain");
}

/// A TurnDone carrying an outdated generation is dropped wholesale: the
/// drain arm stays armed, the loop keeps running, and the agent chip is not
/// reassigned from the stale completion's payload.
#[tokio::test]
async fn stale_turn_done_is_dropped_entirely() {
    let sid = "stale-turn-done";
    let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: sid.into(),
            ..Default::default()
        })
        .await
        .unwrap();

    let mut chat = ChatView {
        agent: "act".into(),
        ..Default::default()
    };
    let mut running = true;
    let mut cancelled = false;
    let mut drain_pending = false;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);
    let mut turn_gen = 0u64;

    armed_drain(
        sid,
        &store,
        &mut chat,
        &mut running,
        &mut drain_pending,
        &mut cancelled,
        &mut turn_gen,
        &cmd_tx,
        &mut cancel,
        &mut evt_rx,
    )
    .await;

    // The cancelled turn's completion arrives with the OLD generation:
    // cancel_running_turn bumped turn_gen to 1 while the worker had already
    // armed its TurnDone tagged with generation 0.
    turn_gen = 1; // what cancel_running_turn's bump leaves behind
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut skip_next_render = false;
    let mut follow = true;
    let mut notepad: Option<crate::notepad::NotepadView> = None;
    let _ = fold_ui_events(
        Some(UiEvent::TurnDone("plan".into(), 0)),
        &mut chat,
        &store,
        sid,
        &mut queue_items,
        &mut false,
        &mut crate::queue_admitter::AdmitUiState::default(),
        &mut running,
        &mut cancelled,
        &mut drain_pending,
        &mut skip_next_render,
        &mut follow,
        &cmd_tx,
        &mut cancel,
        &mut turn_gen, // 1: mismatch with the event's 0
        &mut evt_rx,
        &mut notepad,
        &mut None,
        &opencoder_session::QuestionHub::new(),
    )
    .await;

    assert!(
        drain_pending,
        "stale TurnDone must not consume the drain arm"
    );
    assert!(running, "stale TurnDone must not flip the loop idle");
    assert_eq!(
        chat.agent, "act",
        "stale TurnDone must not reassign the agent chip from its payload"
    );
    assert!(
        cmd_rx.try_recv().is_err(),
        "stale TurnDone must not re-kick any turn"
    );
    drop(evt_tx);
}

/// Control: a matching-generation TurnDone keeps its full effect (drain
/// re-kick restarts, agent chip reconciles) — the guard narrows stale
/// completions only.
#[tokio::test]
async fn matching_turn_done_still_processes() {
    let sid = "fresh-turn-done";
    let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: sid.into(),
            ..Default::default()
        })
        .await
        .unwrap();

    let mut chat = ChatView {
        agent: "act".into(),
        ..Default::default()
    };
    let mut running = true;
    let mut cancelled = false;
    let mut drain_pending = false;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);
    let mut turn_gen = 0u64;

    armed_drain(
        sid,
        &store,
        &mut chat,
        &mut running,
        &mut drain_pending,
        &mut cancelled,
        &mut turn_gen,
        &cmd_tx,
        &mut cancel,
        &mut evt_rx,
    )
    .await;

    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut skip_next_render = false;
    let mut follow = true;
    let mut notepad: Option<crate::notepad::NotepadView> = None;
    let _ = fold_ui_events(
        Some(UiEvent::TurnDone("act".into(), turn_gen)),
        &mut chat,
        &store,
        sid,
        &mut queue_items,
        &mut false,
        &mut crate::queue_admitter::AdmitUiState::default(),
        &mut running,
        &mut cancelled,
        &mut drain_pending,
        &mut skip_next_render,
        &mut follow,
        &cmd_tx,
        &mut cancel,
        &mut turn_gen, // matches the event's generation
        &mut evt_rx,
        &mut notepad,
        &mut None,
        &opencoder_session::QuestionHub::new(),
    )
    .await;

    assert!(!drain_pending, "matching TurnDone consumes the drain arm");
    assert!(running, "the drain re-kick turn is running");
    assert!(matches!(
        cmd_rx.recv().await,
        Some(UiCmd::ResetCancel(_, _))
    ));
}
