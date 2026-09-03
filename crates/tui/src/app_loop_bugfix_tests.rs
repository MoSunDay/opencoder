//! Focused regression tests for the three TUI bugfixes:
//!   - Issue #1/#2: model-switch marker + env-conflict detection
//!   - Issue #3: stale TurnDone must not stall a newer turn
//!
//! Split out of `app_loop_tests.rs` to keep that file under the 800-line cap.

use super::*;
use crate::chat::ChatView;

// ----- Issue #2: env-conflict detection (pure) -----

/// `OPENCODER_MODEL` silently reverts a just-saved `/model` switch because
/// `Config::load` re-applies `apply_env`. `env_model_override` surfaces that.
#[test]
fn env_model_override_detects_silent_revert() {
    // Env set and effective model differs from the picked one => override.
    assert_eq!(
        env_model_override(
            Some("bigmodel/glm-5.2"),
            "qwen3.8/glm-5.2",
            Some("qwen3.8/glm-5.2"),
        ),
        Some("qwen3.8/glm-5.2".to_string())
    );
    // Env set but effective == intended (env matches the pick) => no override.
    assert_eq!(
        env_model_override(
            Some("bigmodel/glm-5.2"),
            "bigmodel/glm-5.2",
            Some("bigmodel/glm-5.2")
        ),
        None
    );
    // No `model` field in the patch (a `/config` generation-param save) => nothing.
    assert_eq!(env_model_override(None, "x", Some("y")), None);
    // Env unset => no override.
    assert_eq!(env_model_override(Some("a"), "b", None), None);
    // Env empty / whitespace => no override.
    assert_eq!(env_model_override(Some("a"), "b", Some("   ")), None);
}

// ----- Issue #3 (root cause B): stale TurnDone must not stall a newer turn -----

/// After a double-Esc abort the user may submit a new turn before the aborted
/// turn's `TurnDone` is processed. At that moment `running == true` (new turn
/// live) and `cancelled == true` (stale flag). The stale `TurnDone` must clear
/// `cancelled` but must NOT flip `running=false` — otherwise the live turn
/// appears permanently stuck. This locks the invariant the `cancelled`-flag
/// design relies on.
#[tokio::test]
async fn fold_stale_turndone_keeps_newer_turn_running() {
    use opencoder_store::LibsqlStore;

    let store: Arc<dyn opencoder_store::Store> =
        Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView::default();
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut running = true; // a newer turn is live
    let mut cancelled = true; // stale flag from the just-aborted turn
    let mut drain_pending = false;
    let mut skip_next_render = false;
    let mut follow = true;
    let (cmd_tx, _cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);

    let mut notepad: Option<crate::notepad::NotepadView> = None;
    let _flow = fold_ui_events(
        Some(UiEvent::TurnDone("act".into())),
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

    assert!(!cancelled, "stale cancel flag should be cleared");
    assert!(running, "the live (newer) turn must stay running");
}

// ----- drain_pending activation: stranded Queue/Steer must arm recovery -----

/// When `SessionEvent::Done` arrives but the store still has a pending Queue
/// input (e.g. it was stranded by a race), `drain_pending` must be armed so the
/// subsequent `TurnDone` restarts the drain loop instead of going idle.
#[tokio::test]
async fn done_with_pending_queue_arms_drain_pending() {
    use opencoder_store::{Delivery, LibsqlStore, SessionInput, SessionMeta, Store};

    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "drain-test".into(),
            title: Some("test".into()),
            agent: Some("act".into()),
            model: Some("m/g".into()),

            autopilot_mode: None,
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
        })
        .await
        .unwrap();
    // Admit a stranded Queue input.
    store
        .admit_input(&SessionInput {
            seq: None,
            id: "q-1".into(),
            session_id: "drain-test".into(),
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
        "drain-test",
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

    assert!(drain_pending, "stranded queue must arm drain_pending");
    assert!(running, "must NOT go idle while drain_pending is armed");
    assert!(
        !queue_items.is_empty(),
        "queue mirror must reflect the stranded item"
    );
}

/// When `SessionEvent::Done` arrives and the store has NO pending inputs,
/// `drain_pending` stays false and `running` goes to false (normal idle).
#[tokio::test]
async fn done_with_empty_store_goes_idle() {
    use opencoder_store::{LibsqlStore, SessionMeta, Store};

    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "idle-test".into(),
            title: Some("test".into()),
            agent: Some("act".into()),
            model: Some("m/g".into()),

            autopilot_mode: None,
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
        })
        .await
        .unwrap();

    let mut chat = ChatView::default();
    let mut queue_items: Vec<(i64, String)> = Vec::new();
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
        "idle-test",
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

    assert!(!drain_pending, "empty store must NOT arm drain_pending");
    assert!(!running, "must go idle when nothing is pending");
}

// ----- Bug #2: drain_pending restart must check start_turn return value -----

/// When `drain_pending` is armed (a cancelled turn left stranded inputs) and
/// the subsequent `TurnDone` triggers the drain-restart path, but the worker
/// task has already died (its `cmd_rx` receiver is dropped), `start_turn`
/// returns `false`. Without the fix, this `false` was silently discarded,
/// leaving the UI in a permanent "running" spinner state. With the fix, the
/// drain-restart branch calls `worker_dead(chat)` and returns `LoopFlow::Quit`
/// — matching every other `start_turn` call site.
#[tokio::test]
async fn drain_pending_restart_with_dead_worker_quits() {
    use opencoder_store::LibsqlStore;

    let store: Arc<dyn opencoder_store::Store> =
        Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView::default();
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut running = true;
    let mut cancelled = false;
    let mut drain_pending = true; // armed: a cancelled turn left stranded inputs
    let mut skip_next_render = false;
    let mut follow = true;
    let (cmd_tx, cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);

    // Simulate worker death: drop the receiver so every send() fails.
    drop(cmd_rx);

    let mut notepad: Option<crate::notepad::NotepadView> = None;
    let flow = fold_ui_events(
        Some(UiEvent::TurnDone("act".into())),
        &mut chat,
        &store,
        "dead-worker-test",
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
        matches!(flow, LoopFlow::Quit),
        "dead worker must return Quit"
    );
    assert!(
        crate::chat::block_text(&chat).contains("worker stopped"),
        "worker_dead marker must be pushed to chat"
    );
    assert!(
        !drain_pending,
        "drain_pending must be cleared before the start_turn attempt"
    );
}

#[path = "app_loop_bugfix_tests/streaming_and_clock.rs"]
mod streaming_and_clock;
