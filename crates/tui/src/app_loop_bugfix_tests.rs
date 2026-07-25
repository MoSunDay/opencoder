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
        env_model_override(Some("bigmodel/glm-5.2"), "bigmodel/glm-5.2", Some("bigmodel/glm-5.2")),
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
        &mut evt_rx,
    )
    .await;

    assert!(!cancelled, "stale cancel flag should be cleared");
    assert!(running, "the live (newer) turn must stay running");
}
