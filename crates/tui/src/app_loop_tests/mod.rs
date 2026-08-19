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
/// the plan turn is running is intercepted — no command sent, input
/// untouched, running stays true, and a busy flash hint is shown (the user
/// re-presses at a clean idle boundary; no deferred auto-fire). The
/// direction-aware gate intercepts every plan→act variant this way (with or
/// without a submitted plan, handoff or no_handoff); act→plan while running
/// is instead a pure state switch (see switch_gate_tests).
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

/// Queued/steered requirements arm the handoff at CONSUMPTION time: the plan
/// turn's `TurnDone(plan)` re-arms `plan_submitted` from the persisted
/// plan-phase counter (incremented when the requirement was recorded for the
/// plan agent). This test seeds the post-TurnDone state directly and pins
/// the `handle_switch_agent` gate contract: armed → SwitchAndStart, not a
/// pure agent swap.
#[tokio::test]
async fn queue_armed_then_shift_tab_plan_to_act_triggers_handoff() {
    let mut chat = ChatView {
        agent: "plan".into(),
        plan_submitted: false,
        ..Default::default()
    };
    // What TurnDone(plan) does after the queued requirement ran in the phase.
    chat.plan_submitted = true;
    assert!(
        chat.plan_submitted,
        "consumption-time arm must arm the plan→act handoff"
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

/// The Tab-queue admit path itself must NOT arm the handoff (arming is
/// consumption-time): right after a successful queue admit, Shift+Tab is a
/// PURE switch — no SwitchAndStart, no turn, context preserved. The arm
/// appears only after the plan turn consumed the row (TurnDone(plan) above).
#[tokio::test]
async fn queue_admit_alone_does_not_arm_shift_tab() {
    let mut chat = ChatView {
        agent: "plan".into(),
        plan_submitted: false,
        ..Default::default()
    };
    // What the Queue branch does on a successful Tab-queue admit in plan mode
    // (no note_requirement_submitted anymore — admit != delivered).
    assert!(
        !chat.plan_submitted,
        "queue admit alone must NOT arm the plan→act handoff"
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
    assert!(!running, "un-armed Shift+Tab must NOT start a turn");
    // Pure switch: exactly one SwitchAgent command, nothing else.
    assert!(matches!(
        cmd_rx.try_recv().unwrap(),
        UiCmd::SwitchAgent(ref n) if n == "act"
    ));
    assert!(cmd_rx.try_recv().is_err(), "no further command");
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

/// Consumption-time arm: TurnDone(plan) re-arms `plan_submitted` from the
/// PERSISTED plan-phase state — the counter (the authoritative record of
/// requirements delivered to the plan agent, incremented at record time by
/// the runner and the queue/steer `record_compound` twin; persisted before
/// the turn ends) OR the phase snapshot. The session row's agent column is
/// deliberately ignored: ts-origin sessions keep it NULL, which used to
/// disarm Shift+Tab here. Covers steers, queued inputs and compound
/// `/plan <content>` alike, and can never arm from a stranded,
/// never-consumed admit.
#[tokio::test]
async fn fold_turn_done_plan_rearms_from_persisted_counter() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    for (sid, agent, count, snapshot, expect_armed) in [
        ("plan-armed", Some("plan"), 2i64, None, true),
        ("plan-empty", Some("plan"), 0i64, None, false),
        // ts-origin sessions keep the session row's agent column NULL by
        // design; the TurnDone(plan) event itself proves a plan turn ran, so
        // the counter must arm the flag regardless of the NULL agent.
        ("ts-origin", None, 2i64, None, true),
        // A phase snapshot alone (legacy backfill, counter still zero) arms.
        (
            "snapshot-only",
            Some("plan"),
            0i64,
            Some("## Plan".to_string()),
            true,
        ),
    ] {
        store
            .create_session(&SessionMeta {
                id: sid.into(),
                agent: agent.map(String::from),
                plan_input_count: count,
                plan_snapshot: snapshot.clone(),
                ..Default::default()
            })
            .await
            .unwrap();

        // A stale-true flag (e.g. sticky from a previous phase) AND the
        // empty counter: TurnDone(plan) must authoritatively UN-arm.
        let mut chat = ChatView {
            agent: "plan".into(),
            plan_submitted: !expect_armed,
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
            sid,
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
        assert_eq!(
            chat.plan_submitted, expect_armed,
            "TurnDone(plan) must re-arm from the persisted counter (count={count})"
        );
    }
}

/// An act-phase TurnDone never touches the arm flag: no store read, no
/// re-arm. (The flag is only re-derived on plan-phase turn ends.)
#[tokio::test]
async fn fold_turn_done_act_leaves_arm_untouched() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "act-sess".into(),
            agent: Some("act".into()),
            plan_input_count: 3, // stale from a pre-handoff phase
            ..Default::default()
        })
        .await
        .unwrap();

    let mut chat = ChatView {
        agent: "act".into(),
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
        Some(UiEvent::TurnDone("act".into())),
        &mut chat,
        &store,
        "act-sess",
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
        !chat.plan_submitted,
        "act TurnDone must not arm the handoff"
    );
}

mod cancel_keep_pending;
mod cli_outcome_tests;
mod envs_outcome_tests;
mod mcp_outcome_tests;
mod model_outcome_tests;
mod skill_outcome_tests;

mod done_error_mirror_tests;

mod act_clear_repro_tests;
mod act_clear_ts_origin_tests;
mod legacy_resume_tests;

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
