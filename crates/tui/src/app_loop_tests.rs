//! Tests for `app_loop` helpers — extracted to keep `app_loop.rs` under the
//! 800-line cap. Compiled as `#[cfg(test)] mod tests` via `#[path]`.

use super::*;
use crate::chat::ChatView;

// ----- Existing route_paste tests -----

/// No modal open + plain (non-file) text: the main-composer path inserts it
/// verbatim, advances the cursor, and returns `Proceed` (caller falls
/// through rather than `continue`).
#[test]
fn route_paste_into_main_composer_inserts_verbatim_text() {
    let mut model_menu: Option<ModelMenu> = None;
    let mut command_menu: Option<CommandMenu> = None;
    let mut input = String::new();
    let mut idx = 0usize;
    let flow = route_paste(
        "plain text",
        false,
        false,
        &mut model_menu,
        &mut command_menu,
        &mut input,
        &mut idx,
        Path::new("."),
    );
    assert!(matches!(flow, LoopFlow::Proceed));
    assert_eq!(input, "plain text");
    assert_eq!(idx, "plain text".chars().count());
}

/// task picker open (no text field): the paste is swallowed — `Redraw` is
/// returned and the main composer stays untouched.
#[test]
fn route_paste_swallowed_when_task_picker_open() {
    let mut model_menu: Option<ModelMenu> = None;
    let mut command_menu: Option<CommandMenu> = None;
    let mut input = String::new();
    let mut idx = 0usize;
    let flow = route_paste(
        "plain text",
        true,
        false,
        &mut model_menu,
        &mut command_menu,
        &mut input,
        &mut idx,
        Path::new("."),
    );
    assert!(matches!(flow, LoopFlow::Redraw));
    assert!(
        input.is_empty(),
        "main composer must be untouched when a modal swallows the paste"
    );
    assert_eq!(idx, 0);
}

/// cache-salt menu open: same modal-isolation contract — paste swallowed,
/// existing composer contents and cursor preserved.
#[test]
fn route_paste_swallowed_when_cache_salt_menu_open() {
    let mut model_menu: Option<ModelMenu> = None;
    let mut command_menu: Option<CommandMenu> = None;
    let mut input = String::from("kept");
    let mut idx = 2usize;
    let flow = route_paste(
        "plain text",
        false,
        true,
        &mut model_menu,
        &mut command_menu,
        &mut input,
        &mut idx,
        Path::new("."),
    );
    assert!(matches!(flow, LoopFlow::Redraw));
    assert_eq!(input, "kept");
    assert_eq!(idx, 2);
}

// ----- plan→act handoff tests (P0 race-fix) -----

fn plan_view() -> ChatView {
    ChatView {
        agent: "plan".into(),
        plan_submitted: true,
        ..Default::default()
    }
}

/// P0 fix: plan→act while running defers the handoff into `pending_handoff`
/// instead of dropping it or racing the worker.
#[tokio::test]
async fn switch_plan_to_act_while_running_defers_handoff() {
    let mut chat = plan_view();
    let mut running = true;
    let mut follow = true;
    let mut input = "extra text".to_string();
    let mut cursor_idx = 10;
    let mut pending_handoff: Option<String> = None;
    let mut mode_flash: Option<(String, u32)> = None;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut sys_tokens = 0u64;
    let workdir = Path::new(".");
    let active_skill_body: Option<String> = None;

    let outcome = handle_switch_agent(
        "act".into(),
        &mut chat,
        &mut running,
        &mut follow,
        &mut input,
        &mut cursor_idx,
        &mut pending_handoff,
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
    assert_eq!(pending_handoff.as_deref(), Some("extra text"));
    assert!(input.is_empty(), "input should be consumed into handoff");
    assert_eq!(cursor_idx, 0);
    assert!(running, "running should stay true (plan turn still active)");
    assert!(
        mode_flash.as_ref().unwrap().0.contains("pending"),
        "mode flash should show pending; got {:?}",
        mode_flash
    );
    assert!(
        cmd_rx.try_recv().is_err(),
        "no command should be sent while deferring"
    );
}

/// Regression: plan→act while idle triggers the handoff immediately.
#[tokio::test]
async fn switch_plan_to_act_while_idle_triggers_handoff() {
    let mut chat = plan_view();
    let mut running = false;
    let mut follow = false;
    let mut input = "do it".to_string();
    let mut cursor_idx = 5;
    let mut pending_handoff: Option<String> = None;
    let mut mode_flash: Option<(String, u32)> = None;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut sys_tokens = 0u64;
    let workdir = Path::new(".");
    let active_skill_body: Option<String> = None;

    let outcome = handle_switch_agent(
        "act".into(),
        &mut chat,
        &mut running,
        &mut follow,
        &mut input,
        &mut cursor_idx,
        &mut pending_handoff,
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
    assert!(pending_handoff.is_none());
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

/// Non-plan→act switch clears any stale pending_handoff (pure switch).
#[tokio::test]
async fn switch_non_plan_to_act_clears_pending() {
    let mut chat = ChatView {
        agent: "act".into(),
        ..Default::default()
    };
    let mut running = false;
    let mut follow = false;
    let mut input = String::new();
    let mut cursor_idx = 0;
    let mut pending_handoff: Option<String> = Some("stale".into());
    let mut mode_flash: Option<(String, u32)> = None;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut sys_tokens = 0u64;
    let workdir = Path::new(".");
    let active_skill_body: Option<String> = None;

    let outcome = handle_switch_agent(
        "plan".into(),
        &mut chat,
        &mut running,
        &mut follow,
        &mut input,
        &mut cursor_idx,
        &mut pending_handoff,
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
    assert!(pending_handoff.is_none(), "stale pending should be cleared");
    match cmd_rx.try_recv().unwrap() {
        UiCmd::SwitchAgent(ref n) => assert_eq!(n, "plan"),
        _ => panic!("expected SwitchAgent"),
    }
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
    let mut pending_handoff: Option<String> = Some("stale".into());
    let mut mode_flash: Option<(String, u32)> = None;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut sys_tokens = 0u64;
    let workdir = Path::new(".");
    let active_skill_body: Option<String> = None;

    let outcome = handle_switch_agent(
        "act".into(),
        &mut chat,
        &mut running,
        &mut follow,
        &mut input,
        &mut cursor_idx,
        &mut pending_handoff,
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
    assert!(pending_handoff.is_none());
    assert!(!running);
    match cmd_rx.try_recv().unwrap() {
        UiCmd::SwitchAgent(ref n) => assert_eq!(n, "act"),
        _ => panic!("expected SwitchAgent"),
    }
}

// ----- fold_ui_events P0/P1 tests -----

use opencoder_core::Message;
use opencoder_session::SessionEvent;
use opencoder_store::{LibsqlStore, SessionMeta};

/// P0 fix: when a plan→act switch was deferred (pending_handoff set) and the
/// plan turn finishes normally (TurnDone), fold_ui_events fires the handoff.
#[tokio::test]
async fn fold_turndone_with_pending_triggers_handoff() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView {
        agent: "plan".into(),
        plan_submitted: true,
        ..Default::default()
    };
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut running = true;
    let mut cancelled = false;
    let mut drain_pending = false;
    let mut skip_next_render = false;
    let mut follow = true;
    let mut pending_handoff: Option<String> = Some("do it now".into());
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);

    let flow = fold_ui_events(
        Some(UiEvent::TurnDone),
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
        &mut pending_handoff,
        &mut evt_rx,
    )
    .await;

    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(pending_handoff.is_none(), "pending_handoff should be consumed");
    assert!(running, "running should be true again (handoff turn started)");
    // ResetCancel + SwitchAndStart("act", "do it now")
    assert!(matches!(cmd_rx.try_recv().unwrap(), UiCmd::ResetCancel(_)));
    match cmd_rx.try_recv().unwrap() {
        UiCmd::SwitchAndStart(ref n, ref extra) => {
            assert_eq!(n, "act");
            assert_eq!(extra, "do it now");
        }
        _ => panic!("expected SwitchAndStart"),
    }
}

/// Cancel path: if the turn was cancelled (user hit Esc), TurnDone should NOT
/// trigger the pending handoff — cancel = explicit interrupt.
#[tokio::test]
async fn fold_turndone_cancelled_blocks_handoff() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut chat = ChatView {
        agent: "plan".into(),
        plan_submitted: true,
        ..Default::default()
    };
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    // Cancel handler already set running=false before TurnDone arrives.
    let mut running = false;
    let mut cancelled = true;
    let mut drain_pending = false;
    let mut skip_next_render = false;
    let mut follow = true;
    let mut pending_handoff: Option<String> = Some("deferred".into());
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);

    let _flow = fold_ui_events(
        Some(UiEvent::TurnDone),
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
        &mut pending_handoff,
        &mut evt_rx,
    )
    .await;

    assert!(
        pending_handoff.is_some(),
        "pending_handoff should NOT be consumed on cancel"
    );
    assert!(!running, "running should be false after cancelled turn");
    assert!(
        cmd_rx.try_recv().is_err(),
        "no command should be sent when cancelled"
    );
}

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
    let mut pending_handoff: Option<String> = None;
    let (cmd_tx, _cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);

    let _flow = fold_ui_events(
        Some(UiEvent::Session(SessionEvent::TranscriptReset(messages))),
        &mut chat,
        &store,
        "p1-test",
        &mut queue_items,
        &mut running,
        &mut cancelled,
        &mut drain_pending,
        &mut skip_next_render,
        &mut follow,
        &cmd_tx,
        &mut cancel,
        &mut pending_handoff,
        &mut evt_rx,
    )
    .await;

    assert!(
        chat.plan_submitted,
        "plan_submitted must survive TranscriptReset (compaction); \
         this is the P1 regression — without the fix, the replay would \
         reset it to false"
    );
}
