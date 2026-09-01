//! Done/Error queue_items mirror-semantics tests — split out of
//! `app_loop_tests/mod.rs` to keep the aggregator under the 800-line cap.
//! Compiled as part of `#[cfg(test)] mod tests` (via `#[path]`).

use super::*;

use opencoder_store::LibsqlStore;

// ----- Done/Error queue_items mirror semantics -----
//
// Both `Done` and `Error` in `fold_ui_events` re-sync the queue AND steer
// mirrors from the store (authoritative rebuild). On `Done` this normally
// empties them — the store queue is provably empty (claim_one_queued
// returned None before Done was emitted) — but a cancel/interrupt race can
// leave rows pending, in which case `drain_pending` is armed. On `Error`
// the rows must stay VISIBLE (the error path short-circuits run_loop
// before the idle boundary, so items may still be pending in the store and
// will be consumed on the next submit's drain), but `drain_pending` is NOT
// armed — no auto-restart, to avoid error loops. Wiping the mirrors on
// either event would make still-pending input invisible even though it
// survives in the store.

/// Pre-populate the store with one pending Queue input and one pending
/// Steer input, prefill both in-memory mirrors with a stale/extra row, then
/// drive `fold_ui_events` with an `Error` event. The mirrors must be
/// re-synced from the store: the stale row disappears, the real pending
/// rows stay visible (they are consumed on the next submit's drain or a
/// `>` panel drain), `running` flips off and — unlike Done —
/// `drain_pending` stays false (no auto-restart after an error).
#[tokio::test]
async fn fold_error_resyncs_mirrors_from_store() {
    use opencoder_store::{Delivery, LibsqlStore, SessionMeta};
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "test-session".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let q_seq = store
        .admit_input(&crate::app_helpers::mk_input_with_images(
            "test-session",
            Delivery::Queue,
            "queued prompt A",
            None,
            &[],
        ))
        .await
        .unwrap();
    let s_seq = store
        .admit_input(&crate::app_helpers::mk_input_with_images(
            "test-session",
            Delivery::Steer,
            "steer prompt A",
            None,
            &[],
        ))
        .await
        .unwrap();
    let mut chat = ChatView::default();
    // Stale rows that no longer exist in the store (e.g. leftovers from an
    // optimistic temp submit that already reconciled elsewhere).
    let mut queue_items: Vec<(i64, String)> = vec![
        (999, "stale queued row".into()),
        (q_seq, "queued prompt A".into()),
    ];
    chat.steer_items = vec![(998, "stale steer row".into())];
    let mut running = true;
    let mut cancelled = false;
    let mut drain_pending = false;
    let mut skip_next_render = false;
    let mut follow = true;
    let (cmd_tx, _cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);

    let mut notepad: Option<crate::notepad::NotepadView> = None;
    let _flow = fold_ui_events(
        Some(UiEvent::Session(SessionEvent::Error(
            "llm api failure".into(),
        ))),
        &mut chat,
        &store,
        "test-session",
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
        !running,
        "running should flip false on Error (not cancelled, no drain pending)"
    );
    assert!(
        !drain_pending,
        "Error must NOT arm drain_pending — no auto-restart (error-loop guard)"
    );
    assert_eq!(
        queue_items,
        vec![(q_seq, "queued prompt A".into())],
        "queue mirror re-synced from store: stale row gone, real pending \
         row kept for the next drain"
    );
    assert_eq!(
        chat.steer_items,
        vec![(s_seq, "steer prompt A".into())],
        "steer mirror re-synced from store instead of being cleared"
    );
}

/// Counterpart: on `Done` the store queue is provably empty
/// (claim_one_queued returned None before Done was emitted), so the
/// in-memory mirror should be wiped.
#[tokio::test]
async fn fold_done_clears_queue_items() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView::default();
    let mut queue_items: Vec<(i64, String)> = vec![
        (20, "queued prompt C".into()),
        (21, "queued prompt D".into()),
    ];
    let mut running = true;
    let mut cancelled = false;
    let mut drain_pending = false;
    let mut skip_next_render = false;
    let mut follow = true;
    let (cmd_tx, _cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);

    let mut notepad: Option<crate::notepad::NotepadView> = None;
    let _flow = fold_ui_events(
        Some(UiEvent::Session(SessionEvent::Done)),
        &mut chat,
        &store,
        "test-session",
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

    assert!(!running, "running should flip false on Done");
    assert!(
        chat.steer_items.is_empty(),
        "steer_items should be cleared on Done"
    );
    assert!(
        queue_items.is_empty(),
        "queue_items should be cleared on Done — store queue is provably empty"
    );
}

/// When a queued follow-up is consumed at the idle boundary, the handler
/// echoes a `ChatBlock::User` block into the transcript and drops the consumed
/// entry by seq from the pending mirror. The block is NOT pushed at admit
/// time — it only appears when the queued prompt actually starts executing.
#[tokio::test]
async fn fold_queue_consumed_echoes_marker_and_drops_entry() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView::default();
    let mut queue_items: Vec<(i64, String)> = vec![
        (30, "queued prompt X".into()),
        (31, "queued prompt Y".into()),
    ];
    let mut running = true;
    let mut cancelled = false;
    let mut drain_pending = false;
    let mut skip_next_render = false;
    let mut follow = true;
    let (cmd_tx, _cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);

    let before = crate::chat::block_text(&chat);
    let mut notepad: Option<crate::notepad::NotepadView> = None;
    let _flow = fold_ui_events(
        Some(UiEvent::Session(SessionEvent::QueueConsumed {
            seq: 30,
            text: "queued prompt X".into(),
        })),
        &mut chat,
        &store,
        "test-session",
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

    // A ChatBlock::User with the consumed prompt is pushed at consume time.
    assert!(
        crate::chat::block_text(&chat).contains("User:"),
        "QueueConsumed must echo the User tag at consume time"
    );
    assert!(
        crate::chat::block_text(&chat).contains("queued prompt X"),
        "QueueConsumed must echo the consumed prompt body"
    );
    assert_ne!(
        crate::chat::block_text(&chat),
        before,
        "transcript must change after QueueConsumed echoes"
    );
    assert_eq!(
        queue_items.len(),
        1,
        "QueueConsumed must drop only the consumed entry from queue_items"
    );
    assert_eq!(queue_items[0].0, 31, "the unconsumed entry must remain");
}

/// A QueueConsumed whose seq does not match any pending entry must be a
/// no-op for the marker (no spurious marker pushed) while still retaining
/// all entries.
#[tokio::test]
async fn fold_queue_consumed_unknown_seq_is_noop() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView::default();
    let mut queue_items: Vec<(i64, String)> = vec![(40, "queued prompt Z".into())];
    let mut running = true;
    let mut cancelled = false;
    let mut drain_pending = false;
    let mut skip_next_render = false;
    let mut follow = true;
    let (cmd_tx, _cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);

    let before = crate::chat::block_text(&chat);
    let mut notepad: Option<crate::notepad::NotepadView> = None;
    let _flow = fold_ui_events(
        Some(UiEvent::Session(SessionEvent::QueueConsumed {
            seq: 999,
            text: String::new(),
        })),
        &mut chat,
        &store,
        "test-session",
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

    assert_eq!(
        crate::chat::block_text(&chat),
        before,
        "unknown seq must not push a marker"
    );
    assert_eq!(queue_items.len(), 1, "unknown seq must retain all entries");
}

/// Safety: when the turn was cancelled (`cancelled=true`), neither
/// `Done` nor `Error` should touch `queue_items` — the event belongs to
/// a stale turn and items may belong to a fresh turn.
#[tokio::test]
async fn fold_error_when_cancelled_preserves_queue_items() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView::default();
    let mut queue_items: Vec<(i64, String)> = vec![(30, "queued after steer".into())];
    let mut running = true;
    let mut cancelled = true;
    let mut drain_pending = false;
    let mut skip_next_render = false;
    let mut follow = true;
    let (cmd_tx, _cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);

    let mut notepad: Option<crate::notepad::NotepadView> = None;
    let _flow = fold_ui_events(
        Some(UiEvent::Session(SessionEvent::Error("stale".into()))),
        &mut chat,
        &store,
        "test-session",
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
        running,
        "running must stay true when the event is from a cancelled turn"
    );
    assert!(!cancelled, "cancelled flag should be reset to false");
    assert_eq!(
        queue_items.len(),
        1,
        "queue_items must be untouched for a stale (cancelled) Error event"
    );
    assert_eq!(queue_items[0].0, 30);
}

/// A bare control command consumed from the queue echoes NOTHING: empty event
/// text plus a raw mirror row must not resurrect the command as a user block
/// (the mirror entry is still dropped by seq). Legacy raw event text
/// normalizes to the compound tail.
#[tokio::test]
async fn fold_queue_consumed_bare_control_command_echoes_nothing() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView::default();
    let mut queue_items: Vec<(i64, String)> = vec![(30, "/plan".into())];
    let mut running = true;
    let mut cancelled = false;
    let mut drain_pending = false;
    let mut skip_next_render = false;
    let mut follow = true;
    let (cmd_tx, _cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);
    let mut notepad: Option<crate::notepad::NotepadView> = None;

    let _flow = fold_ui_events(
        Some(UiEvent::Session(SessionEvent::QueueConsumed {
            seq: 30,
            text: String::new(),
        })),
        &mut chat,
        &store,
        "test-session",
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
        queue_items.is_empty(),
        "the consumed entry is dropped by seq regardless of echo"
    );
    assert!(
        !crate::chat::block_text(&chat).contains("User:"),
        "a bare control command must not echo a user block"
    );

    // Legacy persisted event carrying the raw compound prefix: the display
    // layer normalizes to the tail — the command token never shows.
    let mut queue_items: Vec<(i64, String)> = vec![(31, "/plan review".into())];
    let _flow = fold_ui_events(
        Some(UiEvent::Session(SessionEvent::QueueConsumed {
            seq: 31,
            text: "/plan review".into(),
        })),
        &mut chat,
        &store,
        "test-session",
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

    let text = crate::chat::block_text(&chat);
    assert!(text.contains("review"), "compound tail echoed: {text}");
    assert!(!text.contains("/plan"), "token suppressed: {text}");
}
