//! Helpers for steering focused subagents from the TUI input line.
//!
//! When a running subagent block is focused, Enter produces a
//! `KeyAction::SubagentSteer` that is processed by [`admit_subagent_steer`].
//! The ">" button on child steer rows calls [`fire_subagent_turn_cancel`] to
//! interrupt the current turn and force immediate steer absorption.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use opencoder_session::SharedCancel;
use opencoder_store::{Delivery, SessionInput, Store};

use crate::chat::{ChatBlock, ChatView};

/// Admit a steer to the focused subagent's child session and push it to the
/// child view's `steer_items` for display. Returns `true` on success.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn admit_subagent_steer(
    store: &Arc<dyn Store>,
    chat: &mut ChatView,
    subagent_focus: Option<usize>,
    text: &str,
    image_uris: &[String],
) -> bool {
    let idx = match subagent_focus {
        Some(i) => i,
        None => return false,
    };
    // Extract child_session_id; only allow when the subagent is still running.
    let child_session_id = match chat.blocks.get(idx) {
        Some(ChatBlock::Subagent {
            child_session_id,
            done: false,
            ..
        }) => child_session_id.clone(),
        _ => return false,
    };
    let clean = text.trim();
    if clean.is_empty() {
        return false;
    }
    let input = SessionInput {
        seq: None,
        id: opencoder_session::runner::new_id(),
        session_id: child_session_id,
        delivery: Delivery::Steer,
        prompt: clean.to_string(),
        images: image_uris.to_vec(),
        admitted_seq: 0,
        promoted_seq: None,
    };
    match store.admit_input(&input).await {
        Ok(seq) => {
            if let Some(ChatBlock::Subagent { view, .. }) = chat.blocks.get_mut(idx) {
                view.steer_items.push((seq, clean.to_string()));
            }
            true
        }
        Err(_) => false,
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
    use opencoder_store::LibsqlStore;
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
        }
    }

    #[tokio::test]
    async fn admit_steer_to_running_subagent() {
        let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
        let child_sid = "sub-test-1";

        // Create child session row so FK is valid.
        store
            .create_session(&opencoder_store::SessionMeta {
                id: child_sid.to_string(),
                title: Some("child".into()),
                agent: Some("explore".into()),
                model: Some("m/g".into()),
                workdir_hash: None,
                task_type: None,
                created_at: 0,
                updated_at: 0,
                summary: None,
                summary_seq: None,
                handoff_seq: None,
                handoff_plan: None,
                skill: None,
            })
            .await
            .unwrap();

        let mut chat = ChatView::default();
        chat.blocks
            .push(make_subagent_block(false, child_sid, "task-1"));

        let ok = admit_subagent_steer(&store, &mut chat, Some(0), "change direction", &[]).await;

        assert!(ok);

        // Verify steer was admitted to child session.
        let pending = store
            .pending_inputs(child_sid, Delivery::Steer)
            .await
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].prompt, "change direction");

        // Verify steer was pushed to child view's steer_items.
        if let ChatBlock::Subagent { view, .. } = &chat.blocks[0] {
            assert_eq!(view.steer_items.len(), 1);
            assert_eq!(view.steer_items[0].1, "change direction");
        } else {
            panic!("expected Subagent block");
        }
    }

    #[tokio::test]
    async fn admit_steer_to_done_subagent_returns_false() {
        let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
        let mut chat = ChatView::default();
        chat.blocks
            .push(make_subagent_block(true, "sub-done", "task-2"));

        let ok = admit_subagent_steer(&store, &mut chat, Some(0), "should be rejected", &[]).await;

        assert!(!ok);
    }

    #[tokio::test]
    async fn admit_steer_with_no_focus_returns_false() {
        let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
        let mut chat = ChatView::default();

        let ok = admit_subagent_steer(&store, &mut chat, None, "no focus", &[]).await;

        assert!(!ok);
    }

    #[tokio::test]
    async fn admit_steer_with_empty_text_returns_false() {
        let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
        let mut chat = ChatView::default();
        chat.blocks
            .push(make_subagent_block(false, "sub-test-2", "task-3"));

        let ok = admit_subagent_steer(&store, &mut chat, Some(0), "   ", &[]).await;

        assert!(!ok);
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
