//! Tests for `app_loop` helpers — extracted to keep `app_loop.rs` under the
//! 800-line cap. Compiled as `#[cfg(test)] mod tests` via `#[path]`.

use super::*;
use crate::chat::ChatView;

// ----- Shared test infrastructure (used by submodules) -----

/// Single process-global lock serializing every test that *reads* the global
/// config / `home_dir()` while a sibling could conceivably touch it. The
/// former env-mutating tests now use thread-local `scoped_config_home`
/// instead (no `std::env::set_var`), so this lock is retained only as a
/// belt-and-suspenders serializer — it is no longer load-bearing for safety.
pub(crate) static HOME_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

// ----- plan→act handoff tests (P0 race-fix) -----

fn plan_view() -> ChatView {
    ChatView {
        agent: "plan".into(),
        plan_submitted: true,
        ..Default::default()
    }
}

/// Regression: plan→act while idle triggers the handoff immediately.
#[tokio::test]
async fn switch_plan_to_act_while_idle_triggers_handoff() {
    let mut chat = plan_view();
    let mut running = false;
    let mut follow = false;
    let mut input = "do it".to_string();
    let mut cursor_idx = 5;
    let mut mode_flash: Option<(String, u32)> = None;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut sys_tokens = 0u64;
    let workdir = Path::new(".");
    let active_skill_body: Option<String> = None;

    let outcome = handle_switch_agent(
        "act".into(),
        false,
        &mut chat,
        &mut running,
        &mut follow,
        &mut input,
        &mut cursor_idx,
        &mut mode_flash,
        0,
        &cmd_tx,
        &mut cancel,
        &mut sys_tokens,
        workdir,
        &active_skill_body,
    )
    .await;

    assert!(matches!(outcome, SwitchOutcome::Proceed));
    assert!(running);
    assert!(follow);
    // ResetCancel + SwitchAndStart
    assert!(matches!(cmd_rx.try_recv().unwrap(), UiCmd::ResetCancel(_)));
    match cmd_rx.try_recv().unwrap() {
        UiCmd::SwitchAndStart(ref n, ref extra) => {
            assert_eq!(n, "act");
            assert_eq!(extra, "do it");
        }
        _ => panic!("expected SwitchAndStart"),
    }
}

/// Regression for the removal of deferred handoff: plan→act Shift+Tab while
/// the plan turn is running is now a complete no-op — no command sent, input
/// untouched, running stays true, and a flash hint is shown. The same no-op
/// covers act→plan and plan→act without a submitted plan while a turn is
/// running (any mode switch is deferred to the next clean idle boundary).
#[tokio::test]
async fn switch_plan_to_act_while_running_is_noop() {
    let mut chat = plan_view();
    let mut running = true;
    let mut follow = true;
    let mut input = "do not lose me".to_string();
    let mut cursor_idx = 14;
    let mut mode_flash: Option<(String, u32)> = None;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut sys_tokens = 0u64;
    let workdir = Path::new(".");
    let active_skill_body: Option<String> = None;

    let outcome = handle_switch_agent(
        "act".into(),
        false,
        &mut chat,
        &mut running,
        &mut follow,
        &mut input,
        &mut cursor_idx,
        &mut mode_flash,
        0,
        &cmd_tx,
        &mut cancel,
        &mut sys_tokens,
        workdir,
        &active_skill_body,
    )
    .await;

    assert!(matches!(outcome, SwitchOutcome::Proceed));
    assert!(
        cmd_rx.try_recv().is_err(),
        "no command should be sent while running"
    );
    assert!(running, "running must stay true (plan turn still active)");
    assert_eq!(input, "do not lose me", "input must be untouched on no-op");
    assert_eq!(cursor_idx, 14, "cursor must be untouched on no-op");
    assert!(
        mode_flash
            .as_ref()
            .map(|(t, _)| t.contains("busy"))
            .unwrap_or(false),
        "mode flash should hint that the switch is deferred while busy; got {:?}",
        mode_flash
    );
}

/// plan→act without a submitted plan is a pure switch (no handoff).
#[tokio::test]
async fn switch_plan_to_act_unsubmitted_is_pure_switch() {
    let mut chat = ChatView {
        agent: "plan".into(),
        plan_submitted: false,
        ..Default::default()
    };
    let mut running = false;
    let mut follow = false;
    let mut input = String::new();
    let mut cursor_idx = 0;
    let mut mode_flash: Option<(String, u32)> = None;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut sys_tokens = 0u64;
    let workdir = Path::new(".");
    let active_skill_body: Option<String> = None;

    let outcome = handle_switch_agent(
        "act".into(),
        false,
        &mut chat,
        &mut running,
        &mut follow,
        &mut input,
        &mut cursor_idx,
        &mut mode_flash,
        0,
        &cmd_tx,
        &mut cancel,
        &mut sys_tokens,
        workdir,
        &active_skill_body,
    )
    .await;

    assert!(matches!(outcome, SwitchOutcome::Proceed));
    assert!(!running);
    match cmd_rx.try_recv().unwrap() {
        UiCmd::SwitchAgent(ref n) => assert_eq!(n, "act"),
        _ => panic!("expected SwitchAgent"),
    }
}

/// Queued requirements count as requirement submissions: a plan-mode Tab-queue
/// arms `plan_submitted` (via `note_requirement_submitted`, called by the app
/// loop's Queue branch), so the subsequent idle Shift+Tab plan→act switch must
/// trigger the handoff (SwitchAndStart → context cleared) rather than a pure
/// agent swap. Pins the wiring contract between the queue admit path and the
/// `handle_switch_agent` gate.
/// Compound `/plan <content>` submitted from **act** mode arms the handoff
/// *deferred*: the app sets `pending_plan_arm`, the runner's async
/// `AgentSwitch("plan")` consumes it to re-arm `plan_submitted` (which the
/// event would otherwise reset). The subsequent idle Shift+Tab must then
/// trigger the handoff (SwitchAndStart) rather than a pure agent swap —
/// keeping the created plan and starting the task.
#[tokio::test]
async fn compound_plan_from_act_armed_then_shift_tab_triggers_handoff() {
    let mut chat = ChatView {
        agent: "act".into(),
        plan_submitted: false,
        ..Default::default()
    };
    // What the Submit/Steer/Queue branches do for `/plan <content>`.
    chat.pending_plan_arm = true;
    // The runner applies the mode switch asynchronously...
    chat.apply(&SessionEvent::AgentSwitch("plan".into()));
    assert!(
        chat.plan_submitted,
        "compound /plan from act must arm the plan→act handoff"
    );

    let mut running = false;
    let mut follow = false;
    let mut input = String::new();
    let mut cursor_idx = 0;
    let mut mode_flash: Option<(String, u32)> = None;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut sys_tokens = 0u64;
    let workdir = Path::new(".");
    let active_skill_body: Option<String> = None;

    let outcome = handle_switch_agent(
        "act".into(),
        false,
        &mut chat,
        &mut running,
        &mut follow,
        &mut input,
        &mut cursor_idx,
        &mut mode_flash,
        0,
        &cmd_tx,
        &mut cancel,
        &mut sys_tokens,
        workdir,
        &active_skill_body,
    )
    .await;

    assert!(matches!(outcome, SwitchOutcome::Proceed));
    assert!(running, "Shift+Tab after an armed plan must start the task");
    // ResetCancel precedes the handoff command (same as the Enter-armed path).
    assert!(matches!(cmd_rx.try_recv().unwrap(), UiCmd::ResetCancel(_)));
    match cmd_rx.try_recv().unwrap() {
        UiCmd::SwitchAndStart(ref n, _) => assert_eq!(n, "act"),
        _ => panic!("expected SwitchAndStart after compound-/plan-armed plan→act switch"),
    }
}

#[tokio::test]
async fn queue_armed_then_shift_tab_plan_to_act_triggers_handoff() {
    let mut chat = ChatView {
        agent: "plan".into(),
        plan_submitted: false,
        ..Default::default()
    };
    // What the Queue branch does on a successful Tab-queue admit in plan mode.
    chat.note_requirement_submitted();
    assert!(
        chat.plan_submitted,
        "queue admit must arm the plan→act handoff"
    );

    let mut running = false;
    let mut follow = false;
    let mut input = String::new();
    let mut cursor_idx = 0;
    let mut mode_flash: Option<(String, u32)> = None;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut sys_tokens = 0u64;
    let workdir = Path::new(".");
    let active_skill_body: Option<String> = None;

    let outcome = handle_switch_agent(
        "act".into(),
        false,
        &mut chat,
        &mut running,
        &mut follow,
        &mut input,
        &mut cursor_idx,
        &mut mode_flash,
        0,
        &cmd_tx,
        &mut cancel,
        &mut sys_tokens,
        workdir,
        &active_skill_body,
    )
    .await;

    assert!(matches!(outcome, SwitchOutcome::Proceed));
    assert!(running);
    // ResetCancel precedes the handoff command (same as the Enter-armed path).
    assert!(matches!(cmd_rx.try_recv().unwrap(), UiCmd::ResetCancel(_)));
    match cmd_rx.try_recv().unwrap() {
        UiCmd::SwitchAndStart(ref n, _) => assert_eq!(n, "act"),
        _ => panic!("expected SwitchAndStart after queued-armed plan→act switch"),
    }
}

// ----- fold_ui_events P0/P1 tests -----

use opencoder_core::Message;
use opencoder_session::SessionEvent;
use opencoder_store::{LibsqlStore, SessionMeta};

/// P1 fix: TranscriptReset (compaction) must NOT reset plan_submitted to false.
#[tokio::test]
async fn fold_transcript_reset_preserves_plan_submitted() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    // Create the session so replay_into_chat's store queries succeed.
    store
        .create_session(&SessionMeta {
            id: "p1-test".into(),
            agent: Some("plan".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    let mut chat = ChatView {
        agent: "plan".into(),
        plan_submitted: true,
        ..Default::default()
    };
    let messages = vec![Message::user("u1", "compacted summary")];
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut running = false;
    let mut cancelled = false;
    let mut drain_pending = false;
    let mut skip_next_render = false;
    let mut follow = true;
    let (cmd_tx, _cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);

    let mut notepad: Option<crate::notepad::NotepadView> = None;
    let _flow = fold_ui_events(
        Some(UiEvent::Session(SessionEvent::TranscriptReset(messages))),
        &mut chat,
        &store,
        "p1-test",
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

    assert!(
        chat.plan_submitted,
        "plan_submitted must survive TranscriptReset (compaction); \
         this is the P1 regression — without the fix, the replay would \
         reset it to false"
    );
}

/// A dropped `AgentSwitch("plan")` (try_send saturation, the documented
/// hazard family) must not leave a stale `pending_plan_arm`: the next
/// TurnDone carries the authoritative agent, and an unconsumed arm at
/// TurnDone(plan) can only mean the switch event was dropped (FIFO channel).
/// Consume the arm there so the compound-/plan handoff still fires, without
/// risking a spurious re-arm of a later plan-mode entry.
#[tokio::test]
async fn fold_turn_done_plan_consumes_stale_pending_arm() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "p2-test".into(),
            agent: Some("act".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    // Compound `/plan <content>` was submitted from act mode; the runner
    // switched to plan but the AgentSwitch event was dropped (try_send).
    let mut chat = ChatView {
        agent: "act".into(),
        pending_plan_arm: true,
        plan_submitted: false,
        ..Default::default()
    };
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut running = false;
    let mut cancelled = false;
    let mut drain_pending = false;
    let mut skip_next_render = false;
    let mut follow = true;
    let (cmd_tx, _cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);

    let mut notepad: Option<crate::notepad::NotepadView> = None;
    let _flow = fold_ui_events(
        Some(UiEvent::TurnDone("plan".into())),
        &mut chat,
        &store,
        "p2-test",
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

    assert_eq!(chat.agent, "plan");
    assert!(
        chat.plan_submitted,
        "TurnDone(plan) must consume a stale pending_plan_arm (dropped AgentSwitch)"
    );
    assert!(
        !chat.pending_plan_arm,
        "the arm must be consumed exactly once"
    );
}

mod cli_outcome_tests;
mod mcp_outcome_tests;
mod envs_outcome_tests;
mod model_outcome_tests;
mod skill_outcome_tests;

// ----- Done/Error queue_items clear tests -----
//
// Regression: `fold_ui_events`'s `Done | Error` handler used to
// unconditionally `queue_items.clear()`. On `Done` this is safe — the
// store queue is provably empty (claim_one_queued returned None before
// Done was emitted). On `Error` it is WRONG: the error path
// short-circuits run_loop before the idle boundary, so queued items may
// still be pending in the store. Wiping the in-memory mirror makes them
// invisible in the UI even though they would be consumed on the next
// drain. The fix only clears `queue_items` on `Done`.

/// Pre-populate `queue_items` with a couple of pending entries (as if a
/// steer was submitted while running, then the fresh drain errored) and
/// drive `fold_ui_events` with an `Error` event. The mirror must survive
/// — `running` flips off but `queue_items` stays intact.
#[tokio::test]
async fn fold_error_does_not_clear_queue_items() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView::default();
    let mut queue_items: Vec<(i64, String)> = vec![
        (10, "queued prompt A".into()),
        (11, "queued prompt B".into()),
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
        Some(UiEvent::Session(SessionEvent::Error(
            "llm api failure".into(),
        ))),
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

    assert!(
        !running,
        "running should flip false on Error (not cancelled, no drain pending)"
    );
    assert!(
        chat.steer_items.is_empty(),
        "steer_items should be cleared on Error"
    );
    assert_eq!(
        queue_items.len(),
        2,
        "queue_items must NOT be cleared on Error — items may still be \
         pending in the store and would be consumed on the next drain"
    );
    assert_eq!(queue_items[0].0, 10);
    assert_eq!(queue_items[1].0, 11);
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

mod display_title_tests;

#[cfg(test)]
#[path = "../app_loop_plan_edit_tests.rs"]
mod plan_edit_tests;

#[cfg(test)]
#[path = "../app_loop_session_only_tests.rs"]
mod session_only_tests;

mod image_paste_tests;

#[cfg(test)]
#[path = "../app_loop_dispatch_cmd_tests/mod.rs"]
mod dispatch_cmd_tests;

#[cfg(test)]
#[path = "../app_loop_slash_action_tests.rs"]
mod slash_action_tests;

#[cfg(test)]
mod switch_gate_tests;
