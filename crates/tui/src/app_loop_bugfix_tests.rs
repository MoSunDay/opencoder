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

    let _flow = fold_ui_events(
        Some(UiEvent::TurnDone("act".into())),
        &mut chat,
        &store,
        "test-session",
        &mut queue_items,
        &mut running,
        &mut cancelled,
        &mut drain_pending,
        &mut skip_next_render,
        &mut follow,
        &cmd_tx,
        &mut cancel,
        &mut evt_rx,
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

    let _flow = fold_ui_events(
        Some(UiEvent::Session(SessionEvent::Done)),
        &mut chat,
        &store,
        "drain-test",
        &mut queue_items,
        &mut running,
        &mut cancelled,
        &mut drain_pending,
        &mut skip_next_render,
        &mut follow,
        &cmd_tx,
        &mut cancel,
        &mut evt_rx,
    )
    .await;

    assert!(drain_pending, "stranded queue must arm drain_pending");
    assert!(running, "must NOT go idle while drain_pending is armed");
    assert!(!queue_items.is_empty(), "queue mirror must reflect the stranded item");
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

    let _flow = fold_ui_events(
        Some(UiEvent::Session(SessionEvent::Done)),
        &mut chat,
        &store,
        "idle-test",
        &mut queue_items,
        &mut running,
        &mut cancelled,
        &mut drain_pending,
        &mut skip_next_render,
        &mut follow,
        &cmd_tx,
        &mut cancel,
        &mut evt_rx,
    )
    .await;

    assert!(!drain_pending, "empty store must NOT arm drain_pending");
    assert!(!running, "must go idle when nothing is pending");
}

// ----- Bug #7: sys_tokens updated before plan→act noop early return -----

/// When the user presses Shift+Tab (plan→act) while a plan turn is still
/// running AND the plan was already submitted, the switch is a no-op: the
/// agent stays in plan mode and only a "plan running…" flash is shown.
///
/// Previously `*sys_tokens` (the context-meter baseline) was overwritten with
/// the *act*-mode system-prompt token count *before* the no-op early return,
/// corrupting the meter for the remainder of the running plan turn. This test
/// locks the fix: `sys_tokens` must stay at its pre-call plan-mode baseline.
#[tokio::test]
async fn plan_running_noop_does_not_corrupt_sys_tokens() {
    let mut chat = ChatView {
        agent: "plan".into(),
        plan_submitted: true,
        ..ChatView::default()
    };

    let mut running = true;
    let mut follow = true;
    let mut input = String::new();
    let mut cursor_idx = 0usize;
    let mut mode_flash: Option<(String, u32)> = None;
    let anim_tick = 7u32;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let workdir = std::path::Path::new(".");
    let active_skill_body: Option<String> = None;

    // Pick a sentinel baseline that is guaranteed to differ from the act-mode
    // system-prompt token count, so the assertion is meaningful.
    let act_tokens = sys_tokens_for("act", workdir, active_skill_body.as_deref());
    let baseline = if act_tokens == 42_000_000 {
        13_370_042
    } else {
        42_000_000
    };
    assert_ne!(
        baseline, act_tokens,
        "test setup: sentinel must differ from act-mode token count"
    );
    let mut sys_tokens = baseline;

    let outcome = handle_switch_agent(
        "act".into(),
        &mut chat,
        &mut running,
        &mut follow,
        &mut input,
        &mut cursor_idx,
        &mut mode_flash,
        anim_tick,
        &cmd_tx,
        &mut cancel,
        &mut sys_tokens,
        workdir,
        &active_skill_body,
    )
    .await;

    // The no-op path returns Proceed without switching.
    assert!(matches!(outcome, SwitchOutcome::Proceed));
    // The agent must remain in plan mode (no switch happened).
    assert_eq!(chat.agent, "plan", "agent must stay in plan mode on the noop");
    // The running flag must be untouched (still running the plan turn).
    assert!(running, "running flag must not be cleared by the noop");
    // The flash must announce the plan is still running.
    assert!(
        mode_flash
            .as_ref()
            .is_some_and(|(msg, tick)| msg.contains("plan running") && *tick == anim_tick),
        "mode_flash must show the 'plan running' banner, got {mode_flash:?}"
    );
    // The key assertion: the context-meter baseline is NOT overwritten.
    assert_eq!(
        sys_tokens, baseline,
        "sys_tokens must keep the plan-mode baseline on the noop path (got {sys_tokens}, \
         act-mode count was {act_tokens})"
    );
    // And no switch/start command leaked out on the noop path.
    assert!(
        cmd_rx.try_recv().is_err(),
        "no UiCmd must be sent on the plan-running noop path"
    );
}

// ----- Bug #8: dropped AgentSwitch leaves status chip stale -----

/// `AgentSwitch` is delivered via `forward_event` -> `try_send`, which silently
/// drops the event when the UI channel is completely saturated. Since
/// `chat.agent` is written ONLY by that event, a drop leaves the `[plan]` /
/// `[act]` status chip stuck on the pre-switch mode. The fix: `TurnDone`
/// carries the session's authoritative agent and `fold_ui_events` reconciles
/// `chat.agent` from it (TurnDone is sent via `send().await`, so it always
/// arrives).
#[tokio::test]
async fn turn_done_reconciles_agent_when_agent_switch_dropped() {
    use opencoder_store::LibsqlStore;

    let store: Arc<dyn opencoder_store::Store> =
        Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView {
        agent: "plan".into(),
        ..ChatView::default()
    };
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut running = true;
    let mut cancelled = false;
    let mut drain_pending = false;
    let mut skip_next_render = false;
    let mut follow = true;
    let (cmd_tx, _cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);

    let _flow = fold_ui_events(
        Some(UiEvent::TurnDone("act".into())),
        &mut chat,
        &store,
        "test-session",
        &mut queue_items,
        &mut running,
        &mut cancelled,
        &mut drain_pending,
        &mut skip_next_render,
        &mut follow,
        &cmd_tx,
        &mut cancel,
        &mut evt_rx,
    )
    .await;

    assert_eq!(
        chat.agent, "act",
        "TurnDone must reconcile chat.agent to the authoritative session agent"
    );
}

/// Companion: `handle_switch_agent` optimistically sets `chat.agent` so the chip
/// is correct immediately — for non-turning switches (Alt+Tab) that emit no
/// TurnDone, and so a subsequent TranscriptReset rebuild uses the right agent.
#[tokio::test]
async fn handle_switch_agent_sets_agent_optimistically() {
    let mut chat = ChatView {
        agent: "plan".into(),
        ..ChatView::default()
    };
    let mut running = false;
    let mut follow = true;
    let mut input = String::new();
    let mut cursor_idx = 0usize;
    let mut mode_flash: Option<(String, u32)> = None;
    let anim_tick = 3u32;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut sys_tokens = 0u64;
    let workdir = std::path::Path::new(".");
    let active_skill_body: Option<String> = None;

    let outcome = handle_switch_agent(
        "act".into(),
        &mut chat,
        &mut running,
        &mut follow,
        &mut input,
        &mut cursor_idx,
        &mut mode_flash,
        anim_tick,
        &cmd_tx,
        &mut cancel,
        &mut sys_tokens,
        workdir,
        &active_skill_body,
    )
    .await;

    assert!(matches!(outcome, SwitchOutcome::Proceed));
    assert_eq!(
        chat.agent, "act",
        "handle_switch_agent must optimistically set chat.agent before the \
         worker confirms via AgentSwitch"
    );
    assert!(
        matches!(cmd_rx.try_recv(), Ok(UiCmd::SwitchAgent(n)) if n == "act"),
        "a non-turning switch must still send SwitchAgent to the worker"
    );
}

// ----- Status-bar task clock (false→true baseline snap) -----

/// A new turn starts: `running` goes `false → true`. The task clock is
/// never reset at a turn boundary — the accumulated task time survives. The
/// dt baseline is snapped to now so the idle gap is excluded.
#[test]
fn tick_clock_does_not_reset_task_on_turn_start() {
    let mut prev = false;
    let mut last = Instant::now();
    let mut task = 999_999u64; // leftover from a prior turn

    tick_clock(true, &mut prev, &mut last, &mut task);

    assert_eq!(
        task, 999_999,
        "turn start must NOT reset the task clock (reset happens only on new task submission)"
    );
    assert!(prev, "prev_running tracks running after the call");
}

/// Within a single running turn, consecutive ticks accumulate real wall-clock
/// time. Sleeps long enough that dt is deterministically positive (sub-ms
/// noise cannot mask it).
#[test]
fn tick_clock_accumulates_task_while_running() {
    let mut prev = false;
    let mut last = Instant::now();
    let mut task = 0u64;

    tick_clock(true, &mut prev, &mut last, &mut task);
    assert_eq!(task, 0, "baseline snap makes the first tick accumulate ~0");

    std::thread::sleep(std::time::Duration::from_millis(20));
    tick_clock(true, &mut prev, &mut last, &mut task);

    assert!(
        task > 0,
        "consecutive running ticks must accumulate; task={}",
        task
    );
    assert!(
        task < 5_000,
        "single tick accumulation must be small; task={}",
        task
    );
}

/// The task clock survives turn boundaries: it freezes on `true -> false`,
/// does not advance during idle, and keeps accumulating when a new turn
/// starts.
#[test]
fn tick_clock_preserves_task_across_turn_end_and_idle() {
    let mut prev = false;
    let mut last = Instant::now();
    let mut task = 0u64;

    // Turn 1: accumulate.
    tick_clock(true, &mut prev, &mut last, &mut task);
    std::thread::sleep(std::time::Duration::from_millis(20));
    tick_clock(true, &mut prev, &mut last, &mut task);
    assert!(task > 0, "task time should accumulate during turn 1");
    let after_turn1 = task;

    // Turn 1 ends: task time must NOT reset.
    tick_clock(false, &mut prev, &mut last, &mut task);
    assert_eq!(task, after_turn1, "task time must not change on turn end");

    // Idle gap: task time must not advance while idle.
    std::thread::sleep(std::time::Duration::from_millis(20));
    tick_clock(false, &mut prev, &mut last, &mut task);
    assert_eq!(task, after_turn1, "task time must not advance while idle");

    // Turn 2 starts: task time keeps accumulating from the preserved value.
    tick_clock(true, &mut prev, &mut last, &mut task);
    assert_eq!(task, after_turn1, "task time preserved across turn boundary");

    std::thread::sleep(std::time::Duration::from_millis(20));
    tick_clock(true, &mut prev, &mut last, &mut task);
    assert!(
        task > after_turn1,
        "task time must grow during turn 2; task={}, after_turn1={}",
        task,
        after_turn1
    );
}

/// `false -> true` snaps the dt baseline so a long idle gap between turns is
/// never charged to the task clock.
#[test]
fn tick_clock_false_to_true_excludes_idle_gap() {
    let mut prev = false;
    let mut last = Instant::now();
    let mut task = 0u64;

    // First turn accumulates some task time.
    tick_clock(true, &mut prev, &mut last, &mut task);
    std::thread::sleep(std::time::Duration::from_millis(20));
    tick_clock(true, &mut prev, &mut last, &mut task);
    let after_turn1 = task;
    assert!(task > 0, "turn 1 must accumulate task time");

    // Idle: a tick that advances wall-clock but not the task clock.
    std::thread::sleep(std::time::Duration::from_millis(20));
    tick_clock(false, &mut prev, &mut last, &mut task);
    assert_eq!(task, after_turn1, "idle tick must not accumulate");

    // Turn 2 starts after an idle gap: the baseline is snapped to now, so the
    // whole idle stretch is excluded from the very first tick of turn 2.
    tick_clock(true, &mut prev, &mut last, &mut task);
    assert_eq!(
        task, after_turn1,
        "false→true must snap the baseline so the idle gap is not counted"
    );

    // Only subsequent running ticks charge the clock again.
    std::thread::sleep(std::time::Duration::from_millis(20));
    tick_clock(true, &mut prev, &mut last, &mut task);
    assert!(
        task > after_turn1,
        "task time must grow after the turn-2 baseline snap"
    );
}

