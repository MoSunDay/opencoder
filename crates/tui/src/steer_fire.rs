//! Steer dispatch from `app.rs`'s submit paths. Two divergent routes:
//!
//! - **Keyboard Enter** (`KeyAction::Steer`) -> [`admit_keyboard_steer`]:
//!   persist + push onto the pending panel WITHOUT interrupting the running
//!   turn. The steer is absorbed at the next idle/turn boundary by the runner.
//!   Deliberately takes no turn_cancel, so it is structurally incapable of
//!   firing an interrupt.
//!
//! - **Mouse `>` button** (`MouseOutcome::SteerSubmit`) ->
//!   [`fire_steer_interrupt`]: `steer_dispatch::resolve` + `fire_turn_cancel`,
//!   immediately interrupting the running turn.
//!
//! Extracted from `app.rs` so both submit paths share one home and the line
//! budget is respected.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use opencoder_session::SharedCancel;
use opencoder_store::{Delivery, Store};
use tokio_util::sync::CancellationToken;

use crate::app_helpers::{mk_input_with_images, snapshot_image_uris};
use crate::chat::ChatView;
use super::steer_dispatch;

/// Admit a steer submitted via the keyboard Enter path: persist it to the
/// store and push it onto the pending steer panel, WITHOUT interrupting the
/// running turn.
///
/// This takes **no** `turn_cancel` — by construction the keyboard path cannot
/// fire a turn interrupt. The running turn is left to finish naturally and the
/// admitted steer is absorbed at the next idle/turn boundary (runner
/// `claim_steers` / late-steer peek). To interrupt immediately the user clicks
/// the `>` button, which routes through [`fire_steer_interrupt`].
///
/// Snapshots and consumes `pending_images` only on a successful store write,
/// so an attached image is never silently dropped on a store error. Returns
/// the store seq if admitted.
pub(crate) async fn admit_keyboard_steer(
    store: &Arc<dyn Store>,
    session_id: &str,
    clean: &str,
    display: &str,
    pending_images: &mut Vec<(String, String)>,
    chat: &mut ChatView,
) -> Option<i64> {
    let image_uris = snapshot_image_uris(pending_images);
    let input = mk_input_with_images(
        session_id,
        Delivery::Steer,
        clean,
        Some(display.to_string()),
        &image_uris,
    );
    let seq = store.admit_input(&input).await.ok()?;
    pending_images.clear();
    chat.steer_items.push((seq, display.to_string()));
    Some(seq)
}

/// Resolve the steer action and fire the appropriate interrupt.
///
/// Returns the resolved [`steer_dispatch::Action`] so the caller can handle
/// `StartTurn` (which requires async `start_turn` and mutable state not
/// available in this synchronous helper).
pub(crate) fn fire_steer_interrupt(
    running: bool,
    child_cancels: &Arc<Mutex<HashMap<String, CancellationToken>>>,
    turn_cancel: &SharedCancel,
    chat: &ChatView,
) -> steer_dispatch::Action {
    // fire_child_cancels both checks AND cancels children.
    let has_children = running && opencoder_session::fire_child_cancels(child_cancels);
    let action = steer_dispatch::resolve(
        running,
        has_children,
        !chat.steer_items.is_empty(),
    );
    match action {
        steer_dispatch::Action::SteerParent
        | steer_dispatch::Action::CancelChildrenAndSteer => {
            opencoder_session::fire_turn_cancel(turn_cancel);
        }
        _ => {}
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tokio_util::sync::CancellationToken;

    fn fresh_cancel() -> SharedCancel {
        Arc::new(Mutex::new(CancellationToken::new()))
    }

    fn empty_cancels() -> Arc<Mutex<HashMap<String, CancellationToken>>> {
        Arc::new(Mutex::new(HashMap::new()))
    }

    // Core wiring test: a running parent with a pending steer must resolve to
    // SteerParent AND actually fire the shared turn_cancel token. The `>` button
    // (`SteerSubmit`) routes through `fire_steer_interrupt`; the keyboard Enter
    // path (`KeyAction::Steer`) does NOT (it admits without interrupting).
    #[test]
    fn running_parent_with_steer_fires_turn_cancel() {
        let turn_cancel = fresh_cancel();
        let child_cancels = empty_cancels();
        let mut chat = ChatView::default();
        chat.steer_items.push((1, "stop now".into()));

        let action = fire_steer_interrupt(
            true,
            &child_cancels,
            &turn_cancel,
            &chat,
        );

        assert_eq!(action, steer_dispatch::Action::SteerParent);
        assert!(
            turn_cancel.lock().unwrap().is_cancelled(),
            "running parent with a pending steer must fire the turn_cancel"
        );
    }

    // G2 guard: a running parent with live children AND a pending steer must
    // fire turn_cancel (interrupt the parent turn) so the steer is absorbed in
    // one click. Previously this path only cancelled children and the user had to
    // click `>` a second time.
    #[test]
    fn running_parent_with_children_and_steer_fires_turn_cancel() {
        let turn_cancel = fresh_cancel();
        let child_cancels: Arc<Mutex<HashMap<String, CancellationToken>>> =
            Arc::new(Mutex::new(HashMap::new()));
        child_cancels
            .lock()
            .unwrap()
            .insert("child-1".into(), CancellationToken::new());
        let mut chat = ChatView::default();
        chat.steer_items.push((1, "stop now".into()));

        let action = fire_steer_interrupt(
            true,
            &child_cancels,
            &turn_cancel,
            &chat,
        );

        assert_eq!(action, steer_dispatch::Action::CancelChildrenAndSteer);
        assert!(
            turn_cancel.lock().unwrap().is_cancelled(),
            "parent > with children + pending steer must fire turn_cancel"
        );
    }

    // Idle parent (not running) resolves to StartTurn and must NOT fire the
    // turn_cancel — a fresh turn should start, not interrupt a nonexistent one.
    #[test]
    fn idle_parent_resolves_start_turn_without_firing() {
        let turn_cancel = fresh_cancel();
        let child_cancels = empty_cancels();
        let chat = ChatView::default();

        let action = fire_steer_interrupt(
            false,
            &child_cancels,
            &turn_cancel,
            &chat,
        );

        assert_eq!(action, steer_dispatch::Action::StartTurn);
        assert!(
            !turn_cancel.lock().unwrap().is_cancelled(),
            "idle path must not fire the turn_cancel"
        );
    }

    // Running parent with nothing pending is a Noop and must not fire.
    #[test]
    fn running_parent_with_nothing_pending_is_noop() {
        let turn_cancel = fresh_cancel();
        let child_cancels = empty_cancels();
        let chat = ChatView::default();

        let action = fire_steer_interrupt(
            true,
            &child_cancels,
            &turn_cancel,
            &chat,
        );

        assert_eq!(action, steer_dispatch::Action::Noop);
        assert!(
            !turn_cancel.lock().unwrap().is_cancelled(),
            "no-op path must not fire the turn_cancel"
        );
    }

    // Architectural divergence guard: the keyboard Enter path
    // (`KeyAction::Steer`) and the `>` button path (`SteerSubmit`) MUST behave
    // differently when a turn is running with a pending steer.
    //
    //   - `>` button  -> resolve(...) -> SteerParent -> fire_turn_cancel (interrupt)
    //   - Enter key   -> admit only, never call resolve()/fire_steer_interrupt
    //
    // The keyboard path lets the running turn finish naturally; the admitted
    // steer is absorbed at the next idle/turn boundary by the runner (see
    // session `claim_steers` / late-steer peek). This test pins the `>` button
    // resolver so a regression that re-couples Enter to the interrupt path is
    // caught here.
    #[test]
    fn only_button_path_interrupts_running_turn_with_steer() {
        // `>` button: running parent, pending steer, no children -> interrupt.
        assert_eq!(
            steer_dispatch::resolve(true, false, true),
            steer_dispatch::Action::SteerParent,
            "`>` button must resolve to SteerParent (interrupt) when running"
        );
    }

    #[test]
    fn idle_steer_does_not_interrupt() {
        // Neither path fires an interrupt when nothing is running: the `>`
        // button resolves to StartTurn, and the keyboard Enter path would
        // simply admit (a fresh turn is started by the Submit path instead).
        assert_eq!(
            steer_dispatch::resolve(false, false, true),
            steer_dispatch::Action::StartTurn,
            "idle `>` with a pending steer must start a turn, not interrupt"
        );
    }

    // Core invariant (behavioral): the keyboard Enter path admits a steer
    // WITHOUT firing turn_cancel — the running turn finishes naturally and the
    // steer is absorbed at the next idle/turn boundary. This drives the actual
    // keyboard admit seam (`admit_keyboard_steer`) that app.rs's
    // `KeyAction::Steer` arm calls, so the cancel-token contract is observable
    // (not only asserted at the `KeyAction` enum level). It then proves the
    // contrast: the `>` button interrupts the very same running turn.
    #[tokio::test]
    async fn keyboard_enter_admits_steer_without_firing_turn_cancel() {
        use opencoder_store::{LibsqlStore, SessionMeta};

        let turn_cancel = fresh_cancel();
        let store: Arc<dyn Store> =
            Arc::new(LibsqlStore::open_memory().await.unwrap());
        store
            .create_session(&SessionMeta {
                id: "s".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let mut chat = ChatView::default();
        let mut pending_images: Vec<(String, String)> = Vec::new();

        // Keyboard Enter while a turn is running: persist + push, do NOT
        // interrupt. (running is implicit: handle_key returns `KeyAction::Steer`
        // only while a turn is in progress.)
        let seq = admit_keyboard_steer(
            &store,
            "s",
            "stop and rethink",
            "stop and rethink",
            &mut pending_images,
            &mut chat,
        )
        .await;
        assert!(seq.is_some(), "keyboard steer must be admitted to the store");
        assert_eq!(
            chat.steer_items.len(),
            1,
            "keyboard steer must appear on the pending panel"
        );
        assert!(
            !turn_cancel.lock().unwrap().is_cancelled(),
            "keyboard Enter must NOT fire turn_cancel (no interrupt)"
        );

        // Contrast: the `>` button path interrupts the very same running turn.
        let action = fire_steer_interrupt(
            true,
            &empty_cancels(),
            &turn_cancel,
            &chat,
        );
        assert_eq!(action, steer_dispatch::Action::SteerParent);
        assert!(
            turn_cancel.lock().unwrap().is_cancelled(),
            "`>` button must fire turn_cancel (interrupt)"
        );
    }
    // ---- Error-path coverage ----
    use anyhow::Result;
    use opencoder_core::Message;
    use opencoder_store::{
        Delivery, LibsqlStore, SessionEventRecord, SessionFilter, SessionInput,
        SessionListItem, SessionMeta, SessionPatch, SubagentTaskRecord,
    };

    /// A Store wrapper that delegates everything to an inner LibsqlStore
    /// EXCEPT `admit_input`, which always fails. Exercises the documented
    /// store-failure branch of `admit_keyboard_steer`: it must return None and
    /// leave `pending_images` untouched (no silent image drop).
    struct FailingAdmitStore(Arc<LibsqlStore>);

    #[async_trait::async_trait]
    impl Store for FailingAdmitStore {
        fn backend_name(&self) -> &'static str { self.0.backend_name() }
        async fn create_session(&self, m: &SessionMeta) -> Result<()> { self.0.create_session(m).await }
        async fn get_session(&self, id: &str) -> Result<Option<SessionMeta>> { self.0.get_session(id).await }
        async fn list_sessions(&self, f: &SessionFilter) -> Result<Vec<SessionListItem>> { self.0.list_sessions(f).await }
        async fn update_session(&self, id: &str, p: &SessionPatch) -> Result<()> { self.0.update_session(id, p).await }
        async fn delete_session(&self, id: &str) -> Result<()> { self.0.delete_session(id).await }
        async fn clear_other_sessions(&self, k: &str) -> Result<u64> { self.0.clear_other_sessions(k).await }
        async fn append_message(&self, sid: &str, m: &Message) -> Result<i64> { self.0.append_message(sid, m).await }
        async fn append_messages(&self, sid: &str, m: &[Message]) -> Result<Vec<i64>> { self.0.append_messages(sid, m).await }
        async fn load_messages(&self, sid: &str) -> Result<Vec<Message>> { self.0.load_messages(sid).await }
        async fn last_message_seq(&self, sid: &str) -> Result<i64> { self.0.last_message_seq(sid).await }
        async fn admit_input(&self, _input: &SessionInput) -> Result<i64> {
            Err(anyhow::anyhow!("simulated store failure"))
        }
        async fn pending_inputs(&self, sid: &str, d: Delivery) -> Result<Vec<SessionInput>> { self.0.pending_inputs(sid, d).await }
        async fn promote_inputs(&self, sid: &str, up: i64, d: Delivery) -> Result<Vec<i64>> { self.0.promote_inputs(sid, up, d).await }
        async fn promote_next_queued(&self, sid: &str) -> Result<Option<i64>> { self.0.promote_next_queued(sid).await }
        async fn claim_next_queue(&self, sid: &str) -> Result<Option<(i64, SessionInput)>> { self.0.claim_next_queue(sid).await }
        async fn delete_input(&self, id: i64) -> Result<()> { self.0.delete_input(id).await }
        async fn swap_input_order(&self, sid: &str, a: i64, b: i64) -> Result<()> { self.0.swap_input_order(sid, a, b).await }
        async fn append_events(&self, ev: &[SessionEventRecord]) -> Result<Vec<i64>> { self.0.append_events(ev).await }
        async fn events_after(&self, sid: &str, s: i64) -> Result<Vec<SessionEventRecord>> { self.0.events_after(sid, s).await }
        async fn last_event_seq(&self, sid: &str) -> Result<i64> { self.0.last_event_seq(sid).await }
        async fn create_subagent_task(&self, r: &SubagentTaskRecord) -> Result<()> { self.0.create_subagent_task(r).await }
        async fn complete_subagent_task(&self, id: &str, res: &str, ok: bool) -> Result<()> { self.0.complete_subagent_task(id, res, ok).await }
        async fn list_subagent_tasks(&self, pid: &str) -> Result<Vec<SubagentTaskRecord>> { self.0.list_subagent_tasks(pid).await }
        async fn get_subagent_task(&self, id: &str) -> Result<Option<SubagentTaskRecord>> { self.0.get_subagent_task(id).await }
        async fn cancel_subagent_task(&self, id: &str) -> Result<()> { self.0.cancel_subagent_task(id).await }
    }

    // When the store write fails, admit_keyboard_steer must return None AND
    // preserve pending_images (the snapshot is taken first, but clear() only
    // runs after a successful write). No image is silently dropped.
    #[tokio::test]
    async fn store_failure_returns_none_and_preserves_images() {
        let inner = Arc::new(LibsqlStore::open_memory().await.unwrap());
        inner
            .create_session(&SessionMeta { id: "s".into(), ..Default::default() })
            .await
            .unwrap();
        let store: Arc<dyn Store> = Arc::new(FailingAdmitStore(inner));

        let mut chat = ChatView::default();
        let mut pending_images = vec![("img.png".to_string(), "data".to_string())];

        let seq = admit_keyboard_steer(
            &store, "s", "stop", "stop", &mut pending_images, &mut chat,
        )
        .await;

        assert!(seq.is_none(), "store failure must return None");
        assert_eq!(
            pending_images.len(),
            1,
            "pending_images must survive a store failure (no silent image drop)"
        );
        assert!(
            chat.steer_items.is_empty(),
            "steer panel must not be mutated on store failure"
        );
    }

}
