//! App-loop wiring of the tagged `SidecarUsage` frame (session
//! `runner/sidecar.rs` → TUI `/sidecar`): the frame must reach
//! `ChatView::apply` through `fold_ui_events` unchanged, land on the main
//! task's cost (`tokens_total`) and leave the status-bar ctx
//! (`real_context_tokens`) on the parent's own latest round. The original bug
//! surfaced exactly here: after a `/sidecar` question the MAIN status bar
//! jumped to the sidecar's window size.

use super::*;

/// Fold every event through the real `fold_ui_events` batch path (one drained
/// `Session` frame per batch) on a view whose sidecar panel is open the way
/// `/sidecar` leaves it. Returns the view plus the loop flags a cost-only
/// frame must not move: `(running, cancelled, skip_next_render)`.
async fn fold_through_app_loop(events: &[SessionEvent]) -> (ChatView, bool, bool, bool) {
    use opencoder_store::{SessionMeta, TursoStore};

    let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "sidecar-usage-loop".into(),
            agent: Some("act".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    let mut chat = ChatView::default();
    let (tx, _rx) = tokio::sync::mpsc::channel::<crate::sidecar_ui::SidecarCmd>(1);
    crate::sidecar_ui::enter_panel(&mut chat, &tx);
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
    let mut turn_gen = 0u64;

    macro_rules! fold {
        ($ev:expr) => {
            fold_ui_events(
                $ev,
                &mut chat,
                &opencoder_core::Config::default(),
                &store,
                "sidecar-usage-loop",
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
                &mut turn_gen,
                &mut evt_rx,
                &mut notepad,
                &mut None,
                &opencoder_session::QuestionHub::new(),
                &mut crate::reset_rebuild::ResetRebuildState::default(),
            )
            .await
        };
    }

    for ev in events {
        let flow = fold!(Some(UiEvent::Session(ev.clone())));
        assert!(
            matches!(flow, LoopFlow::Proceed),
            "a sidecar cost frame is not a loop-control boundary: {ev:?}"
        );
    }
    (chat, running, cancelled, skip_next_render)
}

/// Happy path: parent round, then a sidecar question. The sidecar's spend
/// joins the cost, the status-bar ctx stays on the parent's own window, and
/// the batch is neither an idle boundary nor a render skip.
#[tokio::test]
async fn sidecar_usage_batch_adds_cost_and_keeps_the_parent_ctx() {
    let (chat, running, cancelled, skip_next_render) = fold_through_app_loop(&[
        SessionEvent::LlmUsage {
            total_tokens: 1_000,
            input_tokens: 900,
            output_tokens: 100,
        },
        SessionEvent::SidecarStart {
            id: "sidecar-1".into(),
            question: "这段代码做什么?".into(),
        },
        SessionEvent::SidecarUsage {
            id: "sidecar-1".into(),
            total_tokens: 9_000,
            input_tokens: 8_900,
            output_tokens: 100,
        },
    ])
    .await;
    assert_eq!(
        chat.tokens_total, 10_000,
        "sidecar spend lands on the main task"
    );
    assert_eq!(
        chat.real_context_tokens,
        Some(1_000),
        "status-bar ctx must stay on the parent's latest round"
    );
    assert!(
        chat.sidecar.as_ref().is_some_and(|p| p.id == "sidecar-1"),
        "the open panel adopts the conversation"
    );
    assert!(
        running && !cancelled,
        "a sidecar frame is not an idle/cancel boundary"
    );
    assert!(!skip_next_render, "the cost batch must stay paint-eligible");
}

/// Edge: a sidecar round before ANY parent round leaves ctx unknown (`—`),
/// never the sidecar's window — the pre-fix symptom on an empty main task.
#[tokio::test]
async fn sidecar_usage_batch_before_any_parent_round_keeps_ctx_unknown() {
    let (chat, running, cancelled, skip_next_render) =
        fold_through_app_loop(&[SessionEvent::SidecarUsage {
            id: "sidecar-1".into(),
            total_tokens: 9_000,
            input_tokens: 8_900,
            output_tokens: 100,
        }])
        .await;
    assert_eq!(
        chat.tokens_total, 9_000,
        "cost still lands on the main task"
    );
    assert_eq!(chat.real_context_tokens, None, "ctx stays unknown");
    assert!(running && !cancelled, "no idle boundary");
    assert!(!skip_next_render, "paint-eligible");
}
