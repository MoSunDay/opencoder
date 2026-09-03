use super::*;

// ----- Thinking visibility: reasoning streams live into the step -----

/// Flattened text of the first step group's first step (test-side mirror of
/// `chat::steps::span_text`, which is private to the `chat` module).
fn first_step_thinking_text(chat: &ChatView) -> String {
    chat.blocks
        .iter()
        .find_map(|b| match b {
            crate::chat::ChatBlock::StepGroup { steps, .. } => Some(steps[0].thinking_raw.clone()),
            _ => None,
        })
        .expect("expected a step group")
}

#[tokio::test]
async fn reasoning_deltas_render_first_frame_then_coalesce_hidden_updates() {
    use opencoder_store::LibsqlStore;

    let store: Arc<dyn opencoder_store::Store> =
        Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView::default();
    let mut queue_items = Vec::new();
    let mut running = true;
    let mut cancelled = false;
    let mut drain_pending = false;
    let mut skip_next_render = false;
    let mut follow = true;
    let (cmd_tx, _cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);

    let mut notepad: Option<crate::notepad::NotepadView> = None;
    fold_ui_events(
        Some(UiEvent::Session(SessionEvent::ReasoningDelta(
            "first".into(),
        ))),
        &mut chat,
        &store,
        "thinking-render-test",
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
        !skip_next_render,
        "the first delta opens the streaming step and must render"
    );

    let mut notepad: Option<crate::notepad::NotepadView> = None;
    fold_ui_events(
        Some(UiEvent::Session(SessionEvent::ReasoningDelta(
            " second".into(),
        ))),
        &mut chat,
        &store,
        "thinking-render-test",
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
    // The group already exists after the first frame. A later hidden delta is
    // an O(1) raw append and can skip repainting.
    assert!(
        skip_next_render,
        "a hidden delta-only frame should be coalesced"
    );
    assert_eq!(
        first_step_thinking_text(&chat),
        "first second",
        "both deltas coalesce into the same step's thinking"
    );
}

#[tokio::test]
async fn batched_reasoning_deltas_coalesce_into_one_step() {
    use opencoder_store::LibsqlStore;

    let store: Arc<dyn opencoder_store::Store> =
        Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView::default();
    let mut queue_items = Vec::new();
    let mut running = true;
    let mut cancelled = false;
    let mut drain_pending = false;
    let mut skip_next_render = false;
    let mut follow = true;
    let (cmd_tx, _cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);
    evt_tx
        .send(UiEvent::Session(SessionEvent::ReasoningDelta(
            " second".into(),
        )))
        .await
        .unwrap();

    let mut notepad: Option<crate::notepad::NotepadView> = None;
    fold_ui_events(
        Some(UiEvent::Session(SessionEvent::ReasoningDelta(
            "first".into(),
        ))),
        &mut chat,
        &store,
        "thinking-batch-test",
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
        !skip_next_render,
        "a batch containing reasoning must render the streaming step"
    );
    // No top-level Thinking block exists any more: the batch coalesced both
    // deltas into one closed-by-default step inside the ladder.
    assert_eq!(
        chat.thinking_headers().len(),
        0,
        "live reasoning never creates a top-level Thinking block"
    );
    assert_eq!(first_step_thinking_text(&chat), "first second");
}

// ----- Bug #8: dropped AgentSwitch leaves status chip stale -----

/// `AgentSwitch` is delivered via `forward_event` -> `try_send`, which silently
/// drops the event when the UI channel is completely saturated. Since
/// `chat.agent` is written ONLY by that event, a drop leaves the `[plan]`
/// / `[act]` status chip stuck on the pre-switch agent. The fix: `TurnDone`
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

    assert_eq!(
        chat.agent, "act",
        "TurnDone must reconcile chat.agent to the authoritative session agent"
    );
}

// ----- Status-bar task clock (false→true baseline snap) -----

/// A new task starts: `running` goes `false → true`. The accumulated task
/// time is not reset here; submission owns that reset. The dt baseline is
/// snapped so the preceding idle gap is excluded.
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

/// Within a running task, consecutive ticks accumulate real wall-clock time.
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

/// The task clock freezes while idle and resumes from its preserved total.
#[test]
fn tick_clock_preserves_task_across_turn_end_and_idle() {
    let mut prev = false;
    let mut last = Instant::now();
    let mut task = 0u64;
    tick_clock(true, &mut prev, &mut last, &mut task);
    std::thread::sleep(std::time::Duration::from_millis(20));
    tick_clock(true, &mut prev, &mut last, &mut task);
    assert!(task > 0, "task time should accumulate while running");
    let after_turn1 = task;

    tick_clock(false, &mut prev, &mut last, &mut task);
    assert_eq!(task, after_turn1, "task time must not change on turn end");

    std::thread::sleep(std::time::Duration::from_millis(20));
    tick_clock(false, &mut prev, &mut last, &mut task);
    assert_eq!(task, after_turn1, "task time must not advance while idle");

    tick_clock(true, &mut prev, &mut last, &mut task);
    assert_eq!(
        task, after_turn1,
        "task time preserved across turn boundary"
    );

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
    tick_clock(true, &mut prev, &mut last, &mut task);
    std::thread::sleep(std::time::Duration::from_millis(20));
    tick_clock(true, &mut prev, &mut last, &mut task);
    let after_turn1 = task;
    assert!(task > 0, "turn 1 must accumulate task time");

    std::thread::sleep(std::time::Duration::from_millis(20));
    tick_clock(false, &mut prev, &mut last, &mut task);
    assert_eq!(task, after_turn1, "idle tick must not accumulate");

    tick_clock(true, &mut prev, &mut last, &mut task);
    assert_eq!(
        task, after_turn1,
        "false→true must snap the baseline so the idle gap is not counted"
    );
    std::thread::sleep(std::time::Duration::from_millis(20));
    tick_clock(true, &mut prev, &mut last, &mut task);
    assert!(
        task > after_turn1,
        "task time must grow after the turn-2 baseline snap"
    );
}
