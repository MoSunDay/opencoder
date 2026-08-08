//! Helpers for steering focused subagents from the TUI input line.
//!
//! When a running subagent block is focused, Enter produces a
//! `KeyAction::SubagentSteer` that is processed by [`admit_subagent_steer`].
//! The ">" button on child steer rows calls [`fire_subagent_turn_cancel`] to
//! interrupt the current turn and force immediate steer absorption.
//!
//! Mirrors `steer_fire::admit_keyboard_steer` for the parent: the steer is
//! admitted to the CHILD session (`Delivery::Steer` keyed by the child's
//! `session_id`) and pushed onto the child view's `steer_items`. The parent's
//! steer panel, queue, skill tokens and active turn are all untouched — no
//! `resolve_persist`, no `turn_cancel` firing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use opencoder_session::{SharedCancel, SubagentSteerGate};
use opencoder_store::{Delivery, Store};

use crate::app_helpers::{mk_input_with_images, snapshot_image_uris};
use crate::chat::{ChatBlock, ChatView};

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum AdmitOutcome {
    Admitted,
    NotRunning,
    StoreError(String),
}

/// Apply the local composer/status consequence of an admission result. Keeping
/// this presentation policy beside admission prevents the main event loop from
/// duplicating the restore-on-reject invariant.
pub(crate) fn apply_admit_outcome(
    outcome: AdmitOutcome,
    text: String,
    input: &mut String,
    cursor_idx: &mut usize,
    chat: &mut ChatView,
) {
    let status = match outcome {
        AdmitOutcome::Admitted => return,
        AdmitOutcome::NotRunning => "subagent already finished — steer not submitted".into(),
        AdmitOutcome::StoreError(error) => error,
    };
    *input = text;
    *cursor_idx = input.chars().count();
    chat.status = status;
}

// Adapter at the event-loop boundary: these borrows are independent UI/store
// state slots and grouping them would only create a second state container.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_subagent_steer(
    store: &Arc<dyn Store>,
    child_steer_gates: &Arc<Mutex<HashMap<String, Arc<SubagentSteerGate>>>>,
    chat: &mut ChatView,
    subagent_focus: Option<usize>,
    text: String,
    pending_images: &mut Vec<(String, String)>,
    input: &mut String,
    cursor_idx: &mut usize,
) {
    let outcome = admit_subagent_steer(
        store,
        child_steer_gates,
        chat,
        subagent_focus,
        &text,
        pending_images,
    )
    .await;
    apply_admit_outcome(outcome, text, input, cursor_idx, chat);
}

/// Admit a steer to the focused subagent's child session and push it to the
/// child view's `steer_items` for display.
///
/// Snapshot-and-consume convention matches `steer_fire::admit_keyboard_steer`:
/// the pending images are only cleared after a successful store write, so an
/// attached image is never silently dropped on a store error.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn admit_subagent_steer(
    store: &Arc<dyn Store>,
    child_steer_gates: &Arc<Mutex<HashMap<String, Arc<SubagentSteerGate>>>>,
    chat: &mut ChatView,
    subagent_focus: Option<usize>,
    text: &str,
    pending_images: &mut Vec<(String, String)>,
) -> AdmitOutcome {
    let idx = match subagent_focus {
        Some(i) => i,
        None => return AdmitOutcome::NotRunning,
    };
    // Extract runtime identity; the UI done flag is only a display hint. The
    // gate is the authoritative lifecycle check that closes atomically with
    // the child runner.
    let (child_session_id, task_id) = match chat.blocks.get(idx) {
        Some(ChatBlock::Subagent {
            child_session_id,
            id,
            done: false,
            ..
        }) => (child_session_id.clone(), id.clone()),
        _ => return AdmitOutcome::NotRunning,
    };
    let clean = text.trim();
    if clean.is_empty() {
        return AdmitOutcome::NotRunning;
    }
    let gate = child_steer_gates
        .lock()
        .ok()
        .and_then(|map| map.get(&task_id).cloned());
    let Some(reservation) = gate.and_then(|gate| gate.reserve()) else {
        return AdmitOutcome::NotRunning;
    };
    let image_uris = snapshot_image_uris(pending_images);
    let input = mk_input_with_images(
        &child_session_id,
        Delivery::Steer,
        clean,
        Some(clean.to_string()),
        &image_uris,
    );
    match store.admit_input(&input).await {
        Ok(seq) => {
            if !reservation.commit() {
                return match store.delete_input(seq).await {
                    Ok(()) => AdmitOutcome::NotRunning,
                    Err(e) => AdmitOutcome::StoreError(format!(
                        "subagent ended and steer rollback failed: {e:#}"
                    )),
                };
            }
            if let Some(ChatBlock::Subagent { view, .. }) = chat.blocks.get_mut(idx) {
                view.steer_items.push((seq, clean.to_string()));
            }
            pending_images.clear();
            AdmitOutcome::Admitted
        }
        Err(e) => AdmitOutcome::StoreError(format!("subagent steer admit failed: {e:#}")),
    }
}

/// Fire the focused subagent's turn-cancel token to interrupt its current
/// turn and force immediate steer absorption. No-op if the subagent is done
/// or the token is not registered (e.g. the child hasn't started yet).
pub(crate) fn fire_subagent_turn_cancel(
    child_turn_cancels: &Arc<Mutex<HashMap<String, SharedCancel>>>,
    chat: &ChatView,
    subagent_focus: Option<usize>,
) {
    let task_id = match subagent_focus.and_then(|idx| chat.blocks.get(idx)) {
        Some(ChatBlock::Subagent {
            id, done: false, ..
        }) => id.clone(),
        _ => return,
    };
    if let Ok(map) = child_turn_cancels.lock() {
        if let Some(token) = map.get(&task_id).cloned() {
            if let Ok(g) = token.lock() {
                g.cancel();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{ChatBlock, ChatView};
    use opencoder_store::{LibsqlStore, SessionMeta};
    use tokio_util::sync::CancellationToken;

    fn make_subagent_block(done: bool, child_session_id: &str, task_id: &str) -> ChatBlock {
        ChatBlock::Subagent {
            id: task_id.to_string(),
            child_session_id: child_session_id.to_string(),
            kind: "explore".to_string(),
            prompt: "test prompt".to_string(),
            view: ChatView::default(),
            done,
            ok: !done,
            cancelled: false,
            summary: String::new(),
            started_at_ms: 0,
            elapsed_ms: None,
        }
    }

    async fn memory_store() -> Arc<dyn Store> {
        Arc::new(LibsqlStore::open_memory().await.unwrap())
    }

    fn gates(task_id: &str) -> Arc<Mutex<HashMap<String, Arc<SubagentSteerGate>>>> {
        let gates = Arc::new(Mutex::new(HashMap::new()));
        gates
            .lock()
            .unwrap()
            .insert(task_id.to_string(), SubagentSteerGate::new());
        gates
    }

    #[tokio::test]
    async fn admit_steer_to_running_subagent() {
        let store = memory_store().await;
        let child_sid = "sub-test-1";

        // Create child session row so FK is valid.
        store
            .create_session(&SessionMeta {
                id: child_sid.to_string(),
                ..Default::default()
            })
            .await
            .unwrap();

        let mut chat = ChatView::default();
        chat.blocks
            .push(make_subagent_block(false, child_sid, "task-1"));
        let mut pending_images = vec![("img.png".to_string(), "data".to_string())];
        let gates = gates("task-1");

        let outcome = admit_subagent_steer(
            &store,
            &gates,
            &mut chat,
            Some(0),
            "change direction",
            &mut pending_images,
        )
        .await;

        assert_eq!(outcome, AdmitOutcome::Admitted);

        // Verify steer was admitted to CHILD session (not the parent).
        let pending = store
            .pending_inputs(child_sid, Delivery::Steer)
            .await
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].prompt, "change direction");
        assert_eq!(pending[0].session_id, child_sid);

        // Verify steer was pushed to child view's steer_items.
        if let ChatBlock::Subagent { view, .. } = &chat.blocks[0] {
            assert_eq!(view.steer_items.len(), 1);
            assert_eq!(view.steer_items[0].1, "change direction");
        } else {
            panic!("expected Subagent block");
        }

        // Success consumes the attached images.
        assert!(pending_images.is_empty(), "images consumed on success");
    }

    #[tokio::test]
    async fn admit_steer_to_done_subagent_returns_false() {
        let store = memory_store().await;
        let mut chat = ChatView::default();
        chat.blocks
            .push(make_subagent_block(true, "sub-done", "task-2"));
        let mut pending_images = vec![("img.png".to_string(), "data".to_string())];

        let outcome = admit_subagent_steer(
            &store,
            &gates("task-2"),
            &mut chat,
            Some(0),
            "should be rejected",
            &mut pending_images,
        )
        .await;

        assert_eq!(outcome, AdmitOutcome::NotRunning);
        // Rejected without consuming images.
        assert_eq!(pending_images.len(), 1);
    }

    #[tokio::test]
    async fn admit_steer_with_no_focus_returns_false() {
        let store = memory_store().await;
        let mut chat = ChatView::default();

        let outcome = admit_subagent_steer(
            &store,
            &gates("unused"),
            &mut chat,
            None,
            "no focus",
            &mut Vec::new(),
        )
        .await;

        assert_eq!(outcome, AdmitOutcome::NotRunning);
    }

    #[tokio::test]
    async fn admit_steer_with_empty_text_returns_false() {
        let store = memory_store().await;
        let mut chat = ChatView::default();
        chat.blocks
            .push(make_subagent_block(false, "sub-test-2", "task-3"));

        let outcome = admit_subagent_steer(
            &store,
            &gates("task-3"),
            &mut chat,
            Some(0),
            "   ",
            &mut Vec::new(),
        )
        .await;

        assert_eq!(outcome, AdmitOutcome::NotRunning);
    }

    #[tokio::test]
    async fn store_failure_preserves_pending_images() {
        let store = memory_store().await;
        let mut chat = ChatView::default();
        chat.blocks
            .push(make_subagent_block(false, "sub-no-row", "task-fk"));
        let mut pending_images = vec![("img.png".to_string(), "data".to_string())];
        let gates = gates("task-fk");

        // No child session row exists -> admit_input hits the FK constraint
        // and fails. The steer must be rejected WITHOUT dropping the images.
        let outcome = admit_subagent_steer(
            &store,
            &gates,
            &mut chat,
            Some(0),
            "lost steer",
            &mut pending_images,
        )
        .await;

        assert!(matches!(outcome, AdmitOutcome::StoreError(_)));
        assert_eq!(
            pending_images.len(),
            1,
            "pending_images must survive a store failure (no silent image drop)"
        );
        if let ChatBlock::Subagent { view, .. } = &chat.blocks[0] {
            assert!(
                view.steer_items.is_empty(),
                "child steer panel must not be mutated on store failure"
            );
        }
    }

    #[tokio::test]
    async fn closed_gate_rejects_without_writing_or_consuming_images() {
        let store = memory_store().await;
        let child_sid = "sub-closed";
        store
            .create_session(&SessionMeta {
                id: child_sid.to_string(),
                ..Default::default()
            })
            .await
            .unwrap();
        let gates = gates("task-closed");
        gates
            .lock()
            .unwrap()
            .get("task-closed")
            .unwrap()
            .force_close();
        let mut chat = ChatView::default();
        chat.blocks
            .push(make_subagent_block(false, child_sid, "task-closed"));
        let mut images = vec![("img.png".into(), "data".into())];

        let outcome =
            admit_subagent_steer(&store, &gates, &mut chat, Some(0), "too late", &mut images).await;

        assert_eq!(outcome, AdmitOutcome::NotRunning);
        assert_eq!(images.len(), 1);
        assert!(store
            .pending_inputs(child_sid, Delivery::Steer)
            .await
            .unwrap()
            .is_empty());
    }

    #[test]
    fn fire_turn_cancel_noop_for_done_subagent() {
        let registry: Arc<Mutex<HashMap<String, SharedCancel>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let chat = ChatView::default();
        // No subagent blocks — should be a no-op.
        fire_subagent_turn_cancel(&registry, &chat, None);
    }

    #[test]
    fn fire_turn_cancel_fires_for_running_subagent() {
        let registry: Arc<Mutex<HashMap<String, SharedCancel>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let token: SharedCancel = Arc::new(Mutex::new(CancellationToken::new()));
        registry
            .lock()
            .unwrap()
            .insert("task-fire".to_string(), token.clone());

        let mut chat = ChatView::default();
        chat.blocks
            .push(make_subagent_block(false, "sub-fire", "task-fire"));

        fire_subagent_turn_cancel(&registry, &chat, Some(0));

        assert!(token.lock().unwrap().is_cancelled());
    }

    #[test]
    fn fire_turn_cancel_noop_when_token_not_registered() {
        let registry: Arc<Mutex<HashMap<String, SharedCancel>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let mut chat = ChatView::default();
        chat.blocks
            .push(make_subagent_block(false, "sub-no-token", "task-missing"));

        // Should be a no-op — no panic, no effect.
        fire_subagent_turn_cancel(&registry, &chat, Some(0));
    }
}
