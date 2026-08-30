//! A steer or queued input actually taking effect (`SteerConsumed` /
//! `QueueConsumed`) re-derives the parent `[act]` chip's task-plan highlight
//! from the consumed input text: a `$task-plan` token in it is newly
//! activated by the runner's `record_compound` at the consumption boundary,
//! so the chip lights up yellow exactly like an idle `$task-plan` submit
//! would. Any other consumed input (plain text, or a token naming a
//! different skill) reverts the chip to the plain accent hue. These tests
//! pin the `fold_ui_events` side of that contract.

use super::*;

async fn fold_fixture() -> (
    ChatView,
    Arc<dyn Store>,
    mpsc::Sender<UiCmd>,
    mpsc::Receiver<UiEvent>,
    CancellationToken,
) {
    let store: Arc<dyn Store> =
        Arc::new(opencoder_store::LibsqlStore::open_memory().await.unwrap());
    let (cmd_tx, _cmd_rx) = mpsc::channel::<UiCmd>(64);
    let (_evt_tx, evt_rx) = mpsc::channel::<UiEvent>(64);
    let chat = ChatView::default();
    (chat, store, cmd_tx, evt_rx, CancellationToken::new())
}

macro_rules! fold {
    ($chat:expr, $store:expr, $cmd_tx:expr, $evt_rx:expr, $cancel:expr, $ev:expr, $flag:expr) => {
        fold_ui_events(
            $ev,
            &mut $chat,
            &$store,
            "plan-chip-test",
            &mut Vec::new(),
            $flag,
            &mut crate::queue_admitter::AdmitUiState::default(),
            &mut true,
            &mut false,
            &mut false,
            &mut false,
            &mut true,
            &mut $cmd_tx,
            &mut $cancel,
            &mut $evt_rx,
            &mut None,
            &mut None,
            &opencoder_session::QuestionHub::new(),
        )
        .await
    };
}

#[tokio::test]
async fn queue_consumed_plain_text_clears_the_plan_chip_highlight() {
    let (mut chat, store, mut cmd_tx, mut evt_rx, mut cancel) = fold_fixture().await;
    let mut plan_flag = true;

    fold!(
        chat,
        store,
        cmd_tx,
        evt_rx,
        cancel,
        Some(UiEvent::Session(SessionEvent::QueueConsumed {
            seq: 7,
            text: "queued follow-up".into(),
        })),
        &mut plan_flag
    );
    assert!(
        !plan_flag,
        "a queued plain-text input taking effect must revert the chip hue"
    );
}

#[tokio::test]
async fn steer_consumed_plain_text_clears_the_plan_chip_highlight() {
    let (mut chat, store, mut cmd_tx, mut evt_rx, mut cancel) = fold_fixture().await;
    let mut plan_flag = true;

    fold!(
        chat,
        store,
        cmd_tx,
        evt_rx,
        cancel,
        Some(UiEvent::Session(SessionEvent::SteerConsumed {
            seq: 3,
            text: "steered follow-up".into(),
        })),
        &mut plan_flag
    );
    assert!(
        !plan_flag,
        "a steered plain-text input taking effect must revert the chip hue"
    );
}

#[tokio::test]
async fn queue_consumed_task_plan_token_lights_the_chip() {
    let (mut chat, store, mut cmd_tx, mut evt_rx, mut cancel) = fold_fixture().await;
    let mut plan_flag = false;

    fold!(
        chat,
        store,
        cmd_tx,
        evt_rx,
        cancel,
        Some(UiEvent::Session(SessionEvent::QueueConsumed {
            seq: 7,
            text: "$task-plan plan the migration".into(),
        })),
        &mut plan_flag
    );
    assert!(
        plan_flag,
        "a consumed queued $task-plan input activates the skill at the \
         consumption boundary and must light the chip yellow"
    );
}

#[tokio::test]
async fn steer_consumed_task_plan_token_lights_the_chip() {
    let (mut chat, store, mut cmd_tx, mut evt_rx, mut cancel) = fold_fixture().await;
    let mut plan_flag = false;

    fold!(
        chat,
        store,
        cmd_tx,
        evt_rx,
        cancel,
        Some(UiEvent::Session(SessionEvent::SteerConsumed {
            seq: 3,
            text: "$task-plan update the plan first".into(),
        })),
        &mut plan_flag
    );
    assert!(
        plan_flag,
        "a consumed steered $task-plan input must light the chip yellow"
    );
}

/// A compound consumed input carrying `task-plan` among other tokens lights
/// the chip: any hit arms the highlight.
#[tokio::test]
async fn queue_consumed_compound_tokens_light_on_any_task_plan_hit() {
    let (mut chat, store, mut cmd_tx, mut evt_rx, mut cancel) = fold_fixture().await;
    let mut plan_flag = false;

    fold!(
        chat,
        store,
        cmd_tx,
        evt_rx,
        cancel,
        Some(UiEvent::Session(SessionEvent::QueueConsumed {
            seq: 9,
            text: "$review then $task-plan then wrap up".into(),
        })),
        &mut plan_flag
    );
    assert!(plan_flag, "any $task-plan token in a compound input lights the chip");
}

/// A consumed input naming only a *different* skill must NOT light the chip
/// (the old unconditional-revert behavior still holds for it).
#[tokio::test]
async fn queue_consumed_other_skill_token_keeps_the_chip_plain() {
    let (mut chat, store, mut cmd_tx, mut evt_rx, mut cancel) = fold_fixture().await;
    let mut plan_flag = true;

    fold!(
        chat,
        store,
        cmd_tx,
        evt_rx,
        cancel,
        Some(UiEvent::Session(SessionEvent::QueueConsumed {
            seq: 11,
            text: "$review this diff".into(),
        })),
        &mut plan_flag
    );
    assert!(
        !plan_flag,
        "a consumed non-plan skill input must revert the chip hue"
    );
}
