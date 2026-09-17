//! `/compact` rekick regression: the worker's Compact arm must end a
//! successful compaction with a terminal `SessionEvent::Done` (web parity).
//! This locks the full recovery chain across both layers:
//!   Done -> app_loop re-syncs pending rows from the store and arms
//!   `drain_pending` -> TurnDone restarts the drain loop with an empty
//!   prompt -> the runner consumes the stranded queued row at the idle
//!   boundary (`QueueConsumed`), leaving the store queue empty.
//! Before the fix the Compact arm emitted only `TurnDone`, so inputs
//! admitted during the compaction turn stranded in the store forever.

use super::*;

use crate::worker::process_cmd;

/// Compact success + a queued input admitted mid-turn: Done must arm
/// `drain_pending`, TurnDone must rekick a drain turn, and that turn's
/// runner must consume the queued row.
#[tokio::test]
async fn compact_done_rekicks_drain_and_consumes_pending_queue() {
    use opencoder_store::{Delivery, LibsqlStore, SessionInput, SessionMeta, Store};

    let sid = "compact-rekick";
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: sid.into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let q_seq = store
        .admit_input(&SessionInput {
            seq: None,
            id: "q-1".into(),
            session_id: sid.into(),
            delivery: Delivery::Queue,
            prompt: "stranded prompt".into(),
            images: vec![],
            admitted_seq: 0,
            promoted_seq: None,
            display_text: None,
        })
        .await
        .unwrap();

    let mut chat = ChatView::default();
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut running = true; // the /compact turn is live
    let mut cancelled = false;
    let mut drain_pending = false;
    let mut skip_next_render = false;
    let mut follow = true;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);
    let mut notepad: Option<crate::notepad::NotepadView> = None;

    // 1) The worker's Compact arm ends the compaction turn with Done.
    let _flow = fold_ui_events(
        Some(UiEvent::Session(SessionEvent::Done)),
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
        &mut evt_rx,
        &mut notepad,
        &mut None,
        &opencoder_session::QuestionHub::new(),
    )
    .await;
    assert!(
        drain_pending,
        "Done with a pending queued row must arm drain_pending"
    );
    assert!(running, "must not go idle while drain_pending is armed");
    assert_eq!(
        queue_items,
        vec![(q_seq, "stranded prompt".into())],
        "queue mirror re-synced from the store"
    );

    // 2) TurnDone consumes the arm and restarts the drain loop.
    let _flow = fold_ui_events(
        Some(UiEvent::TurnDone("act".into())),
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
        &mut evt_rx,
        &mut notepad,
        &mut None,
        &opencoder_session::QuestionHub::new(),
    )
    .await;
    assert!(!drain_pending, "TurnDone consumed the drain_pending arm");
    assert!(running, "the rekick turn must be running");
    // start_turn emits ResetCancel first, then the drain prompt.
    let mut saw_reset = false;
    let prompt_cmd = loop {
        match cmd_rx.try_recv() {
            Ok(UiCmd::ResetCancel(_)) => saw_reset = true,
            Ok(UiCmd::Prompt(prompt, images)) => {
                assert!(saw_reset, "ResetCancel must precede the drain prompt");
                assert!(
                    prompt.is_empty() && images.is_empty(),
                    "the drain rekick uses an empty prompt, got {prompt:?}"
                );
                break UiCmd::Prompt(prompt, images);
            }
            Ok(other) => panic!("unexpected cmd while rekick: {other:?}"),
            Err(_) => panic!("TurnDone must send the drain prompt"),
        }
    };

    // 3) The rekick turn's runner consumes the stranded row at the idle
    // boundary — the exact behavior that used to never run.
    let mock = opencoder_llm::MockChatClient::new().with_default(vec![
        opencoder_llm::LlmEvent::Completed {
            text: "drain turn answer".into(),
            tool_calls: Vec::new(),
            usage: None,
        },
    ]);
    let mut sess = opencoder_session::SessionState::new(
        sid,
        opencoder_core::resolve_agent("act").unwrap(),
        opencoder_core::Config::default(),
        Arc::new(mock),
        std::env::temp_dir(),
    );
    sess.store = Some(store.clone());
    assert!(!process_cmd(prompt_cmd, &mut sess, &evt_tx).await);

    let events: Vec<SessionEvent> = std::iter::from_fn(|| evt_rx.try_recv().ok())
        .filter_map(|e| match e {
            UiEvent::Session(sev) => Some(sev),
            _ => None,
        })
        .collect();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::QueueConsumed { seq, .. } if *seq == q_seq)),
        "the drain turn must consume the stranded queued row, got {events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(e, SessionEvent::Done)),
        "the drain turn must end with Done"
    );
    assert!(
        store
            .pending_inputs(sid, Delivery::Queue)
            .await
            .unwrap()
            .is_empty(),
        "the store queue must be empty after the rekick drain turn"
    );
}
