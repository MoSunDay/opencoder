//! A steer or queued input actually taking effect (`SteerConsumed` /
//! `QueueConsumed`) must clear the parent `[act]` chip's task-plan highlight:
//! the yellow marks an armed plan skill, and once the runner consumes a
//! pending input the user has interjected, so the chip reverts to the plain
//! accent hue. These tests pin the `fold_ui_events` side of that contract.

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
async fn queue_consumed_clears_the_plan_chip_highlight() {
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
        "a queued input taking effect must revert the chip hue"
    );
}

#[tokio::test]
async fn steer_consumed_clears_the_plan_chip_highlight() {
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
        "a steered input taking effect must revert the chip hue"
    );
}
