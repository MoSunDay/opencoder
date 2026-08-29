//! Steer dispatch from `app.rs`'s submit paths. Two divergent routes:
//!
//! - **Keyboard Enter** (`KeyAction::Steer`) -> [`admit_keyboard_steer`]:
//!   persist + push onto the pending panel WITHOUT interrupting the running
//!   turn. The steer is absorbed at the next idle/turn boundary by the runner.
//!   Deliberately takes no turn_cancel, so it is structurally incapable of
//!   firing an interrupt. The admit is persistence + panel display only —
//!   delivery happens at the runner's turn boundary, so a stranded,
//!   never-consumed steer row has no side effects.
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

use super::steer_dispatch;
use super::subagent_input;
use crate::app_helpers::{mk_input_with_images, snapshot_image_uris};
use crate::chat::ChatView;

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
    // The steer is only ADMITTED here, not delivered: the runner absorbs it
    // at the next turn boundary. A stranded row that a cancelled/idle drain
    // never absorbs therefore has no side effects.
    Some(seq)
}

/// Flash shown when a keyboard steer submit fails at the store layer.
/// Mirrors `queue_admitter::apply_done`'s failure flash; the raw text stays
/// recoverable via ↑ history because `push_history` runs on every submit.
pub(crate) const STEER_SUBMIT_FAILED_FLASH: &str =
    "⚠ steer submit failed — recover text with ↑ history";

/// Map an admit outcome to the failure flash (None on success).
pub(crate) fn flash_on_admit_failure(seq: Option<i64>) -> Option<&'static str> {
    match seq {
        Some(_) => None,
        None => Some(STEER_SUBMIT_FAILED_FLASH),
    }
}

/// Resolve the steer action and fire the appropriate interrupt.
///
/// Returns the resolved [`steer_dispatch::Action`] so the caller can handle
/// `StartTurn` (which requires async `start_turn` and mutable state not
/// available in this synchronous helper).
pub(crate) fn fire_steer_interrupt(
    subagent_focus: Option<usize>,
    running: bool,
    child_cancels: &Arc<Mutex<HashMap<String, CancellationToken>>>,
    child_turn_cancels: &Arc<Mutex<HashMap<String, SharedCancel>>>,
    turn_cancel: &SharedCancel,
    chat: &ChatView,
) -> steer_dispatch::Action {
    let sub_focused = subagent_focus.is_some();
    // fire_child_cancels both checks AND cancels children. While a running
    // subagent is focused the `>` targets the CHILD's own turn token, so the
    // siblings are left untouched (no cascade).
    let has_children =
        !sub_focused && running && opencoder_session::fire_child_cancels(child_cancels);
    let action = steer_dispatch::resolve(
        sub_focused,
        running,
        has_children,
        !chat.steer_items.is_empty(),
    );
    match action {
        steer_dispatch::Action::Subagent => {
            subagent_input::fire_subagent_turn_cancel(child_turn_cancels, chat, subagent_focus);
        }
        steer_dispatch::Action::SteerParent | steer_dispatch::Action::CancelChildrenAndSteer => {
            opencoder_session::fire_turn_cancel(turn_cancel);
        }
        _ => {}
    }
    action
}

/// Outcome of a `>` button submit: the turn token was fired (steer admitted,
/// turn interrupted) or `StartTurn` — nothing pending, so the caller should
/// start a fresh turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SteerSubmitOutcome {
    StartTurn,
    SteerOnly,
}

/// `>` button submit path: resolve + fire interrupts, and tell the caller
/// whether a fresh turn must be started. Kept here (rather than in `app.rs`)
/// so the sync interrupt logic and its outcome stay next to
/// [`fire_steer_interrupt`].
pub(crate) fn handle_steer_submit(
    subagent_focus: Option<usize>,
    running: bool,
    child_cancels: &Arc<Mutex<HashMap<String, CancellationToken>>>,
    child_turn_cancels: &Arc<Mutex<HashMap<String, SharedCancel>>>,
    turn_cancel: &SharedCancel,
    chat: &ChatView,
) -> SteerSubmitOutcome {
    let action = fire_steer_interrupt(
        subagent_focus,
        running,
        child_cancels,
        child_turn_cancels,
        turn_cancel,
        chat,
    );
    match action {
        steer_dispatch::Action::StartTurn => SteerSubmitOutcome::StartTurn,
        _ => SteerSubmitOutcome::SteerOnly,
    }
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

    fn empty_turn_cancels() -> Arc<Mutex<HashMap<String, SharedCancel>>> {
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
            None,
            true,
            &child_cancels,
            &empty_turn_cancels(),
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
            None,
            true,
            &child_cancels,
            &empty_turn_cancels(),
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
            None,
            false,
            &child_cancels,
            &empty_turn_cancels(),
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
            None,
            true,
            &child_cancels,
            &empty_turn_cancels(),
            &turn_cancel,
            &chat,
        );

        assert_eq!(action, steer_dispatch::Action::Noop);
        assert!(
            !turn_cancel.lock().unwrap().is_cancelled(),
            "no-op path must not fire the turn_cancel"
        );
    }

    // Subagent steer: while a running subagent is focused, `>` must fire ONLY
    // that child's own turn token — the parent turn_cancel and the sibling
    // hard-cancels stay untouched.
    #[test]
    fn focused_running_subagent_fires_only_its_own_turn_token() {
        let parent_turn = fresh_cancel();
        let child_turn = fresh_cancel();
        let child_cancels = empty_cancels();
        let child_turn_cancels: Arc<Mutex<HashMap<String, SharedCancel>>> =
            Arc::new(Mutex::new(HashMap::new()));
        child_turn_cancels
            .lock()
            .unwrap()
            .insert("child-1".into(), child_turn.clone());

        let mut chat = ChatView::default();
        chat.blocks.push(crate::chat::ChatBlock::Subagent {
            id: "child-1".into(),
            child_session_id: "sub-s".into(),
            kind: "explore".into(),
            prompt: "investigate".into(),
            view: ChatView::default(),
            done: false,
            ok: false,
            cancelled: false,
            summary: String::new(),
            started_at_ms: 0,
            elapsed_ms: None,
        });

        let action = fire_steer_interrupt(
            Some(0),
            true,
            &child_cancels,
            &child_turn_cancels,
            &parent_turn,
            &chat,
        );

        assert_eq!(action, steer_dispatch::Action::Subagent);
        assert!(
            child_turn.lock().unwrap().is_cancelled(),
            "focused subagent's own turn token must fire"
        );
        assert!(
            !parent_turn.lock().unwrap().is_cancelled(),
            "parent turn_cancel must NOT fire while a subagent is focused"
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
            steer_dispatch::resolve(false, true, false, true),
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
            steer_dispatch::resolve(false, false, false, true),
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
        let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
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
        assert!(
            seq.is_some(),
            "keyboard steer must be admitted to the store"
        );
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
            None,
            true,
            &empty_cancels(),
            &empty_turn_cancels(),
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
        Delivery, LibsqlStore, SessionEventRecord, SessionFilter, SessionInput, SessionListItem,
        SessionMeta, SessionPatch, SubagentTaskRecord,
    };

    /// A Store wrapper that delegates everything to an inner LibsqlStore
    /// EXCEPT `admit_input`, which always fails. Exercises the documented
    /// store-failure branch of `admit_keyboard_steer`: it must return None and
    /// leave `pending_images` untouched (no silent image drop).
    struct FailingAdmitStore(Arc<LibsqlStore>);

    #[async_trait::async_trait]
    impl Store for FailingAdmitStore {
        fn backend_name(&self) -> &'static str {
            self.0.backend_name()
        }
        async fn create_session(&self, m: &SessionMeta) -> Result<()> {
            self.0.create_session(m).await
        }
        async fn get_session(&self, id: &str) -> Result<Option<SessionMeta>> {
            self.0.get_session(id).await
        }
        async fn list_sessions(&self, f: &SessionFilter) -> Result<Vec<SessionListItem>> {
            self.0.list_sessions(f).await
        }
        async fn update_session(&self, id: &str, p: &SessionPatch) -> Result<()> {
            self.0.update_session(id, p).await
        }
        async fn delete_session(&self, id: &str) -> Result<()> {
            self.0.delete_session(id).await
        }
        async fn clear_other_sessions(&self, k: &str) -> Result<u64> {
            self.0.clear_other_sessions(k).await
        }
        async fn append_message(&self, sid: &str, m: &Message) -> Result<i64> {
            self.0.append_message(sid, m).await
        }
        async fn append_messages(&self, sid: &str, m: &[Message]) -> Result<Vec<i64>> {
            self.0.append_messages(sid, m).await
        }
        async fn load_messages(&self, sid: &str) -> Result<Vec<Message>> {
            self.0.load_messages(sid).await
        }
        async fn last_message_seq(&self, sid: &str) -> Result<i64> {
            self.0.last_message_seq(sid).await
        }
        async fn admit_input(&self, _input: &SessionInput) -> Result<i64> {
            Err(anyhow::anyhow!("simulated store failure"))
        }
        async fn pending_inputs(&self, sid: &str, d: Delivery) -> Result<Vec<SessionInput>> {
            self.0.pending_inputs(sid, d).await
        }
        async fn promote_inputs(&self, sid: &str, up: i64, d: Delivery) -> Result<Vec<i64>> {
            self.0.promote_inputs(sid, up, d).await
        }
        async fn promote_next_queued(&self, sid: &str) -> Result<Option<i64>> {
            self.0.promote_next_queued(sid).await
        }
        async fn claim_next_queue(&self, sid: &str) -> Result<Option<(i64, SessionInput)>> {
            self.0.claim_next_queue(sid).await
        }
        async fn delete_input(&self, id: i64) -> Result<()> {
            self.0.delete_input(id).await
        }
        async fn swap_input_order(&self, sid: &str, a: i64, b: i64) -> Result<()> {
            self.0.swap_input_order(sid, a, b).await
        }
        async fn append_events(&self, ev: &[SessionEventRecord]) -> Result<Vec<i64>> {
            self.0.append_events(ev).await
        }
        async fn events_after(&self, sid: &str, s: i64) -> Result<Vec<SessionEventRecord>> {
            self.0.events_after(sid, s).await
        }
        async fn last_event_seq(&self, sid: &str) -> Result<i64> {
            self.0.last_event_seq(sid).await
        }
        async fn create_subagent_task(&self, r: &SubagentTaskRecord) -> Result<()> {
            self.0.create_subagent_task(r).await
        }
        async fn complete_subagent_task(&self, id: &str, res: &str, ok: bool) -> Result<()> {
            self.0.complete_subagent_task(id, res, ok).await
        }
        async fn list_subagent_tasks(&self, pid: &str) -> Result<Vec<SubagentTaskRecord>> {
            self.0.list_subagent_tasks(pid).await
        }
        async fn get_subagent_task(&self, id: &str) -> Result<Option<SubagentTaskRecord>> {
            self.0.get_subagent_task(id).await
        }
        async fn cancel_subagent_task(&self, id: &str) -> Result<()> {
            self.0.cancel_subagent_task(id).await
        }
    }

    // When the store write fails, admit_keyboard_steer must return None AND
    // preserve pending_images (the snapshot is taken first, but clear() only
    // runs after a successful write). No image is silently dropped.
    #[tokio::test]
    async fn store_failure_returns_none_and_preserves_images() {
        let inner = Arc::new(LibsqlStore::open_memory().await.unwrap());
        inner
            .create_session(&SessionMeta {
                id: "s".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let store: Arc<dyn Store> = Arc::new(FailingAdmitStore(inner));

        let mut chat = ChatView::default();
        let mut pending_images = vec![("img.png".to_string(), "data".to_string())];

        let seq =
            admit_keyboard_steer(&store, "s", "stop", "stop", &mut pending_images, &mut chat).await;

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

    // A keyboard steer is only ADMITTED here: the runner absorbs it at the
    // next turn boundary, where a control command (/sandbox, /act,
    // /act_clear_context) is applied. The admit itself must never touch the
    // agent chip or the transcript.
    #[tokio::test]
    async fn keyboard_steer_admit_does_not_touch_agent_state() {
        let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
        store
            .create_session(&SessionMeta {
                id: "steer-sandbox".into(),
                ..Default::default()
            })
            .await
            .unwrap();

        let mut chat = ChatView {
            agent: "sandbox".into(),
            ..Default::default()
        };
        let mut pending_images = Vec::new();

        let seq = admit_keyboard_steer(
            &store,
            "steer-sandbox",
            "also cover the CLI flag",
            "also cover the CLI flag",
            &mut pending_images,
            &mut chat,
        )
        .await
        .expect("admit must succeed");

        assert_eq!(
            chat.agent, "sandbox",
            "a steer admit must never switch the agent chip"
        );
        assert_eq!(
            chat.steer_items,
            vec![(seq, "also cover the CLI flag".to_string())],
            "steer must be mirrored on the pending panel"
        );
        let pending = store
            .pending_inputs("steer-sandbox", Delivery::Steer)
            .await
            .unwrap();
        assert_eq!(
            pending.len(),
            1,
            "admitted steer must still be pending in the store"
        );
        assert_eq!(pending[0].seq, Some(seq));
    }

    // The panel mirror is agent-agnostic: the same admit path works while the
    // act agent is active.
    #[tokio::test]
    async fn keyboard_steer_admit_works_in_act_mode() {
        let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
        store
            .create_session(&SessionMeta {
                id: "steer-act".into(),
                ..Default::default()
            })
            .await
            .unwrap();

        let mut chat = ChatView {
            agent: "act".into(),
            ..Default::default()
        };
        let mut pending_images = Vec::new();

        let seq = admit_keyboard_steer(
            &store,
            "steer-act",
            "stop exploring",
            "stop exploring",
            &mut pending_images,
            &mut chat,
        )
        .await
        .expect("admit must succeed");

        assert_eq!(chat.agent, "act", "act chip must stay untouched");
        assert_eq!(chat.steer_items.len(), 1, "panel mirror is agent-agnostic");
        assert!(seq > 0);
    }

    // F4 seam: a None admit outcome must map to a non-empty failure flash
    // (never a silent drop); a successful seq maps to None (no flash).
    #[test]
    fn flash_on_admit_failure_none_on_success() {
        assert_eq!(flash_on_admit_failure(Some(7)), None);
    }

    #[test]
    fn flash_on_admit_failure_some_on_store_failure() {
        let flash = flash_on_admit_failure(None).expect("failure must produce a flash");
        assert!(!flash.is_empty());
        assert!(flash.contains("steer"), "flash must name the steer path");
        assert_eq!(flash, STEER_SUBMIT_FAILED_FLASH);
    }
}
