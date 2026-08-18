//! Regression tests for the three "submitted input swallowed" bugs:
//!
//! 1. double-Esc (`cancel_running_turn`) used to DELETE pending steer/queue
//!    rows from the store — the user's submitted input was silently lost.
//!    Cancel now only cancels the current run; pending rows survive in the
//!    store AND in the UI mirrors, aligned with the web `/interrupt`
//!    semantics (cancel ≠ discard input). No auto-restart of the drain: the
//!    user just explicitly cancelled, so the rows wait for the next submit
//!    or a `>` panel drain.
//! 2. an `Error` event used to wipe `chat.steer_items` with no revival —
//!    the mirrors are now re-synced from the store (same authoritative
//!    rebuild as `Done`), minus the drain re-arm.
//! 3. an optimistic temp queue row could be dropped by a Done-triggered
//!    authoritative mirror rebuild before the admitter actor's completion
//!    landed; `reconcile_ok` now re-inserts the real row at the tail.

use super::*;

// ---------------------------------------------------------------------------
// 1. cancel_running_turn keeps pending rows (store + mirrors)
// ---------------------------------------------------------------------------

/// Double-Esc must NOT delete pending rows from the store and must NOT wipe
/// the mirror(s) it is handed: the rows stay visible and are consumed FIFO
/// by the next submit's drain or a `>` panel drain. Two queued rows must
/// come back from the store in admit (FIFO) order.
#[tokio::test]
async fn cancel_running_turn_keeps_pending_rows_in_store_and_mirrors() {
    use opencoder_store::{Delivery, LibsqlStore, SessionMeta};
    let store = LibsqlStore::open_memory().await.unwrap();
    let sid = "cancel-keep";
    store
        .create_session(&SessionMeta {
            id: sid.into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let q1 = store
        .admit_input(&crate::app_helpers::mk_input_with_images(
            sid,
            Delivery::Queue,
            "queued prompt A",
            None,
            &[],
        ))
        .await
        .unwrap();
    let q2 = store
        .admit_input(&crate::app_helpers::mk_input_with_images(
            sid,
            Delivery::Queue,
            "queued prompt B",
            None,
            &[],
        ))
        .await
        .unwrap();
    let s1 = store
        .admit_input(&crate::app_helpers::mk_input_with_images(
            sid,
            Delivery::Steer,
            "steer prompt A",
            None,
            &[],
        ))
        .await
        .unwrap();

    let mut chat = crate::chat::ChatView {
        steer_items: vec![(s1, "steer prompt A".into())],
        ..Default::default()
    };
    let mut cancel = CancellationToken::new();
    let empty_child_maps = || {
        (
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        )
    };
    let (c, t, g) = empty_child_maps();
    let mut child_runtime = crate::worker::ChildRuntimeHandles {
        cancels: std::sync::Arc::new(std::sync::Mutex::new(c)),
        turn_cancels: std::sync::Arc::new(std::sync::Mutex::new(t)),
        steer_gates: std::sync::Arc::new(std::sync::Mutex::new(g)),
    };
    let mut running = true;
    let mut cancelled = false;
    let mut follow = false;

    cancel_running_turn(
        &mut chat,
        &mut cancel,
        &mut child_runtime,
        &mut running,
        &mut cancelled,
        &mut follow,
    )
    .await;

    // Cancel effects on the run itself are unchanged.
    assert!(cancel.is_cancelled(), "the live turn's token is cancelled");
    assert!(!running, "running flips false");
    assert!(cancelled, "cancelled flag set");
    assert!(follow, "follow re-armed so the marker is visible");
    // The mirror handed to cancel survives untouched.
    assert_eq!(
        chat.steer_items,
        vec![(s1, "steer prompt A".into())],
        "steer mirror passed in must survive cancel"
    );
    // Store rows survive for BOTH deliveries…
    let queued = store.pending_inputs(sid, Delivery::Queue).await.unwrap();
    let steered = store.pending_inputs(sid, Delivery::Steer).await.unwrap();
    assert_eq!(queued.len(), 2, "queued rows still pending in the store");
    assert_eq!(steered.len(), 1, "steer row still pending in the store");
    // …and the queue comes back in admitted (FIFO) order.
    assert_eq!(
        queued.iter().map(|i| i.seq).collect::<Vec<_>>(),
        vec![Some(q1), Some(q2)],
        "two queued rows must come back in admit order"
    );
    assert_eq!(
        queued.iter().map(|i| i.prompt.clone()).collect::<Vec<_>>(),
        vec!["queued prompt A".to_string(), "queued prompt B".to_string()],
        "FIFO body order preserved"
    );
}

// ---------------------------------------------------------------------------
// 2. Error event re-syncs the steer mirror from the store
// ---------------------------------------------------------------------------

/// A steer row admitted while running must stay visible in the steer mirror
/// after an `Error` event: the mirror is rebuilt from the store (stale rows
/// dropped, real rows kept) and — unlike `Done` — `drain_pending` stays
/// false so the run does not auto-restart.
#[tokio::test]
async fn fold_error_resyncs_steer_mirror_from_store() {
    use opencoder_store::{Delivery, LibsqlStore, SessionMeta};
    let store: std::sync::Arc<dyn opencoder_store::Store> =
        std::sync::Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "test-session".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let s_seq = store
        .admit_input(&crate::app_helpers::mk_input_with_images(
            "test-session",
            Delivery::Steer,
            "steer me",
            None,
            &[],
        ))
        .await
        .unwrap();

    let mut chat = crate::chat::ChatView {
        // Prefill with a stale row plus the real one, as if an optimistic
        // mirror had drifted from the store.
        steer_items: vec![(998, "stale steer row".into()), (s_seq, "steer me".into())],
        ..Default::default()
    };
    let mut queue_items: Vec<(i64, String)> = vec![(999, "stale queued row".into())];
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
        Some(UiEvent::Session(SessionEvent::Error("boom".into()))),
        &mut chat,
        &store,
        "test-session",
        &mut queue_items,
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
        chat.steer_items,
        vec![(s_seq, "steer me".into())],
        "steer mirror re-synced from the store: stale row dropped, real row kept"
    );
    assert!(
        queue_items.is_empty(),
        "queue mirror rebuilt from an empty store queue"
    );
    assert!(!running, "Error goes idle");
    assert!(
        !drain_pending,
        "Error must NOT arm drain_pending (no auto-restart / error-loop guard)"
    );
}

// ---------------------------------------------------------------------------
// 3. Done-overwrite race: optimistic temp row re-inserted
// ---------------------------------------------------------------------------

/// A Done-triggered authoritative mirror rebuild can drop the optimistic
/// temp row before the admitter actor's completion lands. `reconcile_ok`
/// must re-insert the REAL row at the tail so the queued input stays
/// visible until consumed (FIFO preserved: earlier rows were already
/// present when the rebuild happened).
#[test]
fn reconcile_ok_reinserts_after_done_overwrite_race() {
    use crate::queue_admitter::{reconcile_ok, AdmitReconcile};
    // temp row -1 already overwritten by the Done-triggered rebuild.
    let mut items = vec![(9, "real-b".to_string())];
    assert_eq!(
        reconcile_ok(&mut items, &[], -1, 7, "queued-A"),
        AdmitReconcile::Reinserted
    );
    assert_eq!(
        items,
        vec![(9, "real-b".to_string()), (7, "queued-A".to_string())],
        "real row appended at the tail — the queued input is not swallowed"
    );
}

/// End-to-end through `apply_done`: the same race, but folded via the
/// actor-completion path the UI loop actually uses.
#[test]
fn apply_done_reinserts_after_done_overwrite_race() {
    use crate::queue_admitter::{apply_done, AdmitDone, InflightAdmit};
    let mut st = crate::queue_admitter::AdmitUiState {
        next_temp_seq: -1,
        inflight: vec![InflightAdmit {
            temp_seq: -1,
            images: vec![],
        }],
        consumed: vec![],
    };
    let mut queue_items = vec![(9, "real-b".to_string())];
    let mut pending_images: Vec<(String, String)> = vec![];

    let flash = apply_done(
        &mut st,
        AdmitDone {
            temp_seq: -1,
            result: Ok(7),
            display: "queued-A".into(),
        },
        &mut queue_items,
        &mut pending_images,
    );

    assert!(flash.is_none(), "success path never flashes");
    assert!(st.inflight.is_empty(), "inflight stash consumed");
    assert_eq!(
        queue_items,
        vec![(9, "real-b".to_string()), (7, "queued-A".to_string())],
        "apply_done re-inserts the real row at the tail"
    );
}
