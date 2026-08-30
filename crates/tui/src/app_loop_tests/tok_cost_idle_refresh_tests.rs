//! Turn-final `[tok cost]` frame landing — regression guard for the idle-boundary
//! display-cache refresh wired in `app.rs` (`body_refresh_pending = true` on an
//! idle `Proceed` batch).
//!
//! The turn's trailing `LlmUsage` is display-only: `ChatView::apply` accumulates
//! `tokens_total` but marks nothing body-dirty. The app loop's display cache
//! rebuilds on a 333ms body ticker while `dirty` holds; once the turn goes idle
//! nothing re-arms `dirty`, so the cached snapshot — and with it the corner —
//! could freeze one refresh behind until the next keypress (observed live: the
//! corner showed the previous turn's total after turn end). The app-loop wiring
//! forces one cache refresh at the idle boundary; these tests pin the
//! preconditions that wiring relies on: the turn-final batch sequence
//! (`LlmUsage` → `Done` → `TurnDone`) must (a) accumulate into `tokens_total`,
//! (b) flip `running` off exactly at that boundary, and (c) stay paint-eligible
//! (`skip_next_render == false`) so the refreshed cache actually renders.

use super::*;

/// Drive the turn-final event sequence and pin the idle-boundary contract.
#[tokio::test]
async fn turn_final_usage_batch_is_idle_and_paint_eligible() {
    use opencoder_store::{LibsqlStore, SessionMeta};

    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "tok-cost-idle".into(),
            agent: Some("act".into()),
            ..Default::default()
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

    macro_rules! fold {
        ($ev:expr) => {
            fold_ui_events(
                $ev,
                &mut chat,
                &store,
                "tok-cost-idle",
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
            .await
        };
    }

    // 1) Trailing usage of the turn's last round: display-only accumulation.
    fold!(Some(UiEvent::Session(SessionEvent::LlmUsage {
        total_tokens: 700_000,
        input_tokens: 600_000,
        output_tokens: 100_000,
    })));
    assert_eq!(chat.tokens_total, 700_000, "usage must accumulate");
    assert!(running, "usage precedes the idle boundary");
    assert!(!skip_next_render, "usage batch must stay paint-eligible");

    // 2) Done: the run flips idle here (no stranded inputs -> no drain restart).
    fold!(Some(UiEvent::Session(SessionEvent::Done)));
    assert!(!running, "Done must flip running off at the boundary");
    assert!(!skip_next_render, "Done batch must stay paint-eligible");

    // 3) TurnDone: authoritative idle close; running stays off.
    fold!(Some(UiEvent::TurnDone("act".into())));
    assert!(!running, "TurnDone must leave the app idle");
    assert!(!skip_next_render, "TurnDone batch must stay paint-eligible");
    assert_eq!(
        chat.tokens_total, 700_000,
        "idle boundary must not disturb the accumulated total"
    );
}
