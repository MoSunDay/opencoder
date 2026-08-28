//! Off-loop queue admission for the TUI event loop.
//!
//! `store.admit_input` contends on the store-wide db_lock with the running
//! turn's message flusher, subagent flushers and queue claims, so awaiting it
//! inline in the select-loop key branch freezes rendering and input for the
//! whole queue-wait. The actor spawned here owns that wait; the UI loop stays
//! non-blocking via an optimistic temp row (negative seq) plus completion
//! reconciliation ([`reconcile_ok`] / [`reconcile_err`]) once the real store
//! seq (or the failure) comes back.

use std::sync::Arc;

use opencoder_store::{Delivery, SessionInput, Store};
use tokio::sync::mpsc;

/// Actor request: admit `input`, tag the completion with `temp_seq`, and
/// carry `display` through so a late reconciliation can re-insert the row.
pub struct AdmitReq {
    pub temp_seq: i64,
    pub input: SessionInput,
    pub display: String,
}

/// Actor completion: the store result for the admitted input, plus the
/// display text carried from the request (needed when the optimistic temp
/// row was already dropped by an authoritative mirror rebuild and must be
/// re-inserted — see [`reconcile_ok`]).
pub struct AdmitDone {
    pub temp_seq: i64,
    pub result: anyhow::Result<i64>,
    pub display: String,
}

/// One in-flight optimistic submit's image snapshot (restored on failure).
pub struct InflightAdmit {
    pub temp_seq: i64,
    pub images: Vec<(String, String)>,
}

/// UI-side mutable state for optimistic admission. Plain data; every field is
/// only mutated through the functions in this module.
#[derive(Default)]
pub struct AdmitUiState {
    pub next_temp_seq: i64,
    pub inflight: Vec<InflightAdmit>,
    pub consumed: Vec<i64>,
}

/// Spawn the admission actor. Capacity 32 on both channels; requests are
/// processed serially FIFO so the store's `admitted_seq` ordering matches the
/// UI queue ordering. If the done-send fails the UI is gone — break.
pub fn spawn_admitter(
    store: Arc<dyn Store>,
) -> (mpsc::Sender<AdmitReq>, mpsc::Receiver<AdmitDone>) {
    let (req_tx, mut req_rx) = mpsc::channel::<AdmitReq>(32);
    let (done_tx, done_rx) = mpsc::channel(32);
    tokio::spawn(async move {
        while let Some(req) = req_rx.recv().await {
            let result = store.admit_input(&req.input).await;
            if done_tx
                .send(AdmitDone {
                    temp_seq: req.temp_seq,
                    result,
                    display: req.display,
                })
                .await
                .is_err()
            {
                break; // UI gone; nothing left to reconcile.
            }
        }
    });
    (req_tx, done_rx)
}

/// Optimistically enqueue one queue submit. Strictly non-blocking: the only
/// hand-off is `try_send`, whose only failure modes are the actor being gone
/// or the channel saturated at 32 in-flight admits; on failure the temp row
/// and the stashed images are rolled back.
pub fn submit(
    tx: &mpsc::Sender<AdmitReq>,
    st: &mut AdmitUiState,
    queue_items: &mut Vec<(i64, String)>,
    pending_images: &mut Vec<(String, String)>,
    input: SessionInput,
    display: String,
) -> bool {
    st.next_temp_seq -= 1;
    let temp_seq = st.next_temp_seq;
    let images = std::mem::take(pending_images);
    st.inflight.push(InflightAdmit { temp_seq, images });
    queue_items.push((temp_seq, display.clone()));
    match tx.try_send(AdmitReq {
        temp_seq,
        input,
        display,
    }) {
        Ok(()) => true,
        Err(_) => {
            if let Some(snapshot) = take_inflight(st, temp_seq) {
                *pending_images = snapshot;
            }
            if let Some(pos) = queue_items.iter().position(|(s, _)| *s == temp_seq) {
                queue_items.remove(pos);
            }
            false
        }
    }
}

/// Submit-while-running admission: on success the temp row stays queued for
/// the runner to consume at the idle boundary; on failure [`submit`] has
/// already rolled the temp row and images back. The caller owns `history`, so
/// a failed admit stays recoverable via ↑.
///
/// The input is only ADMITTED here, never delivered: a queued control command
/// (`/sandbox`, `/act`, `/clear_context`) is applied by the runner at the
/// idle boundary, so a stranded row that a cancelled/idle drain never
/// absorbs cannot touch the live transcript.
pub(crate) fn admit_running(
    tx: &mpsc::Sender<AdmitReq>,
    st: &mut AdmitUiState,
    queue_items: &mut Vec<(i64, String)>,
    pending_images: &mut Vec<(String, String)>,
    input: SessionInput,
    display: String,
) -> bool {
    submit(tx, st, queue_items, pending_images, input, display)
}

/// Deferred queue admission for a submission made while a turn is running
/// (Tab-queue, and a Submit that reaches the running state via BackTab's
/// compound `/sandbox …` / `/clear_context …`): the **raw** text is
/// admitted verbatim, `$name` tokens included. Skill resolution, activation
/// and persistence all happen at CONSUMPTION time — the runner's
/// `record_compound` at the idle boundary — never at submit time. (Eager
/// resolution here used to write the `skill_prompt` Arc shared with the
/// in-flight LLM call, so a queued `$skill` armed the `[active skill]`
/// reminder and latent tools in the *still running* turn: the skill "fired"
/// immediately.)
///
/// The queue panel shows the same raw text (what the user typed); the user
/// message the LLM eventually sees is recorded token-stripped by
/// `record_compound`, so the token never reaches the model.
pub(crate) fn handle_queue(
    text: &str,
    tx: &mpsc::Sender<AdmitReq>,
    st: &mut AdmitUiState,
    queue_items: &mut Vec<(i64, String)>,
    pending_images: &mut Vec<(String, String)>,
    session_id: &str,
) {
    let raw = text.trim();
    if raw.is_empty() {
        return;
    }
    // Compound control commands (`/sandbox <content>`,
    // `/clear_context <content>`) are consumed by the runner's
    // control-command intercept: the agent switch / transcript fold is
    // applied and the trailing content runs as the next prompt —
    // consumption time, never submit time.
    let display = raw.to_string();
    // Snapshot BEFORE submit: submit consumes pending_images into the
    // in-flight stash on the success path.
    let input = crate::app_helpers::mk_input_with_images(
        session_id,
        Delivery::Queue,
        raw,
        Some(display.clone()),
        &crate::app_helpers::snapshot_image_uris(pending_images),
    );
    admit_running(tx, st, queue_items, pending_images, input, display);
}

/// Outcome of reconciling a successful completion against the queue mirror.
#[derive(Debug, PartialEq, Eq)]
pub enum AdmitReconcile {
    Replaced,
    DroppedConsumed,
    DroppedDuplicate,
    /// The optimistic temp row was missing (an authoritative mirror rebuild
    /// already overwrote it), so the real row was appended at the tail.
    Reinserted,
    Missing,
}

/// Fold an Ok completion into the queue mirror: rewrite the temp row's seq to
/// the real one, in place. A `QueueConsumed` for `real_seq` that already
/// arrived (or an authoritative mirror rebuild that already installed the
/// row) means the temp row must be dropped, not resurrected. If the temp row
/// is simply gone — a Done-triggered authoritative mirror rebuild can drop
/// the optimistic temp row before the actor completion lands — the real row
/// is re-inserted at the END of the mirror so the queued input stays visible
/// until consumed; tail append preserves FIFO order because every earlier
/// row was already present when the rebuild happened.
pub fn reconcile_ok(
    items: &mut Vec<(i64, String)>,
    consumed: &[i64],
    temp_seq: i64,
    real_seq: i64,
    display: &str,
) -> AdmitReconcile {
    let drop_temp = |items: &mut Vec<(i64, String)>| {
        if let Some(pos) = items.iter().position(|(s, _)| *s == temp_seq) {
            items.remove(pos);
        }
    };
    if consumed.contains(&real_seq) {
        drop_temp(items);
        return AdmitReconcile::DroppedConsumed;
    }
    if items.iter().any(|(s, _)| *s == real_seq) {
        drop_temp(items);
        return AdmitReconcile::DroppedDuplicate;
    }
    match items.iter().position(|(s, _)| *s == temp_seq) {
        Some(pos) => {
            items[pos].0 = real_seq;
            AdmitReconcile::Replaced
        }
        None => {
            items.push((real_seq, display.to_string()));
            AdmitReconcile::Reinserted
        }
    }
}

/// Remove a failed submit's temp row; returns whether it was still present.
pub fn reconcile_err(items: &mut Vec<(i64, String)>, temp_seq: i64) -> bool {
    match items.iter().position(|(s, _)| *s == temp_seq) {
        Some(pos) => {
            items.remove(pos);
            true
        }
        None => false,
    }
}

/// Remove and return the image snapshot stashed for `temp_seq`.
pub fn take_inflight(st: &mut AdmitUiState, temp_seq: i64) -> Option<Vec<(String, String)>> {
    let pos = st.inflight.iter().position(|r| r.temp_seq == temp_seq)?;
    Some(st.inflight.remove(pos).images)
}

/// Best-effort image restore: only into an empty pending buffer, so a failed
/// submit can't clobber images the user attached afterwards.
pub fn restore_images(
    pending: &mut Vec<(String, String)>,
    snapshot: Vec<(String, String)>,
) -> bool {
    if pending.is_empty() && !snapshot.is_empty() {
        *pending = snapshot;
        true
    } else {
        false
    }
}

/// Record a consumed seq; cap the ledger at 128 entries (drop oldest).
pub fn note_consumed(st: &mut AdmitUiState, seq: i64) {
    st.consumed.push(seq);
    let excess = st.consumed.len().saturating_sub(128);
    st.consumed.drain(..excess);
}

/// Apply one actor completion: reconcile the mirror and, on failure, restore
/// the stashed images. Returns a transient flash message on failure.
pub fn apply_done(
    st: &mut AdmitUiState,
    done: AdmitDone,
    queue_items: &mut Vec<(i64, String)>,
    pending_images: &mut Vec<(String, String)>,
) -> Option<&'static str> {
    let AdmitDone {
        temp_seq,
        result,
        display,
    } = done;
    let snapshot = take_inflight(st, temp_seq);
    match result {
        Ok(real_seq) => {
            reconcile_ok(queue_items, &st.consumed, temp_seq, real_seq, &display);
            None
        }
        Err(_) => {
            reconcile_err(queue_items, temp_seq);
            if let Some(images) = snapshot {
                restore_images(pending_images, images);
            }
            Some("⚠ queue submit failed — recover text with ↑ history")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use opencoder_core::Message;
    use opencoder_store::{
        LibsqlStore, SessionEventRecord, SessionFilter, SessionListItem, SessionMeta, SessionPatch,
        SubagentTaskRecord,
    };

    fn mk_input(prompt: &str) -> SessionInput {
        SessionInput {
            seq: None,
            id: "x".into(),
            session_id: "s".into(),
            delivery: Delivery::Queue,
            prompt: prompt.into(),
            images: vec![],
            display_text: None,
            admitted_seq: 0,
            promoted_seq: None,
        }
    }

    #[test]
    fn reconcile_ok_replaces_temp_row_in_place() {
        let mut items = vec![(-1, "a".to_string()), (5, "b".to_string())];
        assert_eq!(
            reconcile_ok(&mut items, &[], -1, 9, "a"),
            AdmitReconcile::Replaced
        );
        assert_eq!(
            items,
            vec![(9, "a".to_string()), (5, "b".to_string())],
            "position preserved"
        );
    }

    #[test]
    fn reconcile_ok_drops_duplicate() {
        let mut items = vec![(-1, "a".to_string()), (9, "b".to_string())];
        assert_eq!(
            reconcile_ok(&mut items, &[], -1, 9, "a"),
            AdmitReconcile::DroppedDuplicate
        );
        assert_eq!(
            items,
            vec![(9, "b".to_string())],
            "real row already present, no duplicate"
        );
    }

    #[test]
    fn reconcile_ok_drops_consumed() {
        let mut items = vec![(-1, "a".to_string())];
        assert_eq!(
            reconcile_ok(&mut items, &[9], -1, 9, "a"),
            AdmitReconcile::DroppedConsumed
        );
        assert!(items.is_empty(), "consumed seq must not be resurrected");
    }

    #[test]
    fn reconcile_ok_missing_reinserts_real_row_at_tail() {
        // Done-triggered authoritative mirror rebuild dropped the optimistic
        // temp row (-1) before the actor completion landed — the real row must
        // be re-inserted at the tail so the queued input stays visible.
        let mut items = vec![(5, "b".to_string())];
        assert_eq!(
            reconcile_ok(&mut items, &[], -1, 9, "queued-A"),
            AdmitReconcile::Reinserted
        );
        assert_eq!(
            items,
            vec![(5, "b".to_string()), (9, "queued-A".to_string())],
            "missing temp row: real row appended at the tail (FIFO preserved)"
        );
    }

    #[test]
    fn reconcile_err_removes_temp_row() {
        let mut items = vec![(-1, "a".to_string()), (5, "b".to_string())];
        assert!(reconcile_err(&mut items, -1));
        assert_eq!(items, vec![(5, "b".to_string())]);
        assert!(
            !reconcile_err(&mut items, -1),
            "absent temp row reports false"
        );
    }

    #[test]
    fn note_consumed_caps_at_128() {
        let mut st = AdmitUiState::default();
        for i in 0..200 {
            note_consumed(&mut st, i);
        }
        assert_eq!(st.consumed.len(), 128);
        assert_eq!(*st.consumed.last().unwrap(), 199);
        assert_eq!(*st.consumed.first().unwrap(), 72, "oldest dropped");
    }

    #[test]
    fn restore_images_only_into_empty_pending() {
        let mut pending = vec![];
        assert!(restore_images(&mut pending, vec![("a".into(), "b".into())]));
        assert_eq!(pending, vec![("a".to_string(), "b".to_string())]);
        let mut pending = vec![("x".into(), "y".into())];
        assert!(!restore_images(
            &mut pending,
            vec![("a".into(), "b".into())]
        ));
        assert_eq!(
            pending,
            vec![("x".to_string(), "y".to_string())],
            "pending unchanged"
        );
    }

    #[test]
    fn submit_round_trip_without_store() {
        let (tx, mut rx) = mpsc::channel(1);
        let mut st = AdmitUiState::default();
        let mut queue_items = vec![];
        let mut pending_images = vec![("img.png".to_string(), "alt".to_string())];
        assert!(submit(
            &tx,
            &mut st,
            &mut queue_items,
            &mut pending_images,
            mk_input("p"),
            "d1".into()
        ));
        assert_eq!(queue_items, vec![(-1, "d1".to_string())]);
        assert!(
            pending_images.is_empty(),
            "images move into the inflight stash"
        );
        assert_eq!(st.inflight.len(), 1);
        assert_eq!(st.inflight[0].temp_seq, -1);
        assert_eq!(st.inflight[0].images.len(), 1);
        let req = rx.try_recv().unwrap();
        assert_eq!(req.temp_seq, -1);
        assert_eq!(req.input.prompt, "p");
        // A second submit allocates the next temp seq (-2).
        let mut pending2 = vec![];
        assert!(submit(
            &tx,
            &mut st,
            &mut queue_items,
            &mut pending2,
            mk_input("q"),
            "d2".into()
        ));
        assert_eq!(queue_items[1], (-2, "d2".to_string()));
    }

    #[test]
    fn submit_rolls_back_on_dead_sender() {
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        let mut st = AdmitUiState::default();
        let mut queue_items = vec![];
        let mut pending_images = vec![("img.png".to_string(), "alt".to_string())];
        assert!(!submit(
            &tx,
            &mut st,
            &mut queue_items,
            &mut pending_images,
            mk_input("p"),
            "d".into()
        ));
        assert!(queue_items.is_empty(), "temp row rolled back");
        assert_eq!(
            pending_images,
            vec![("img.png".to_string(), "alt".to_string())]
        );
        assert!(st.inflight.is_empty(), "stash rolled back");
    }

    /// A successful admit-while-running must NOT touch the live transcript:
    /// the input is only ADMITTED. A queued control command is applied by
    /// the runner at the idle boundary, so a stranded row that a
    /// cancelled/idle drain never absorbs cannot fold the transcript or
    /// switch the agent.
    #[test]
    fn admit_running_success_does_not_apply_control_cmd() {
        let (tx, _rx) = mpsc::channel(1);
        let mut st = AdmitUiState::default();
        let mut queue_items = vec![];
        let mut pending_images = vec![];
        assert!(admit_running(
            &tx,
            &mut st,
            &mut queue_items,
            &mut pending_images,
            mk_input("p"),
            "d".into()
        ));
        assert_eq!(queue_items, vec![(-1, "d".to_string())]);
    }

    /// A failed admit (actor gone / channel saturated) must not count as a
    /// delivered requirement and must leave the mirror fully rolled back.
    #[test]
    fn admit_running_failure_rolls_back() {
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        let mut st = AdmitUiState::default();
        let mut queue_items = vec![];
        let mut pending_images = vec![("img.png".to_string(), "alt".to_string())];
        assert!(!admit_running(
            &tx,
            &mut st,
            &mut queue_items,
            &mut pending_images,
            mk_input("p"),
            "d".into()
        ));
        assert!(queue_items.is_empty(), "temp row rolled back");
        assert_eq!(
            pending_images,
            vec![("img.png".to_string(), "alt".to_string())]
        );
        assert!(st.inflight.is_empty(), "stash rolled back");
    }

    #[tokio::test]
    async fn actor_round_trip_admits_and_reconciles() {
        let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
        store
            .create_session(&SessionMeta {
                id: "s".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let (tx, mut done_rx) = spawn_admitter(Arc::clone(&store) as Arc<dyn Store>);
        let mut st = AdmitUiState::default();
        let mut queue_items = vec![];
        let mut pending_images = vec![("img.png".to_string(), "alt".to_string())];
        assert!(submit(
            &tx,
            &mut st,
            &mut queue_items,
            &mut pending_images,
            mk_input("p"),
            "d".into()
        ));
        let done = done_rx.recv().await.unwrap();
        assert_eq!(done.temp_seq, -1);
        let real_seq = done.result.unwrap();
        assert!(real_seq > 0);
        assert_eq!(
            reconcile_ok(&mut queue_items, &st.consumed, -1, real_seq, "d"),
            AdmitReconcile::Replaced
        );
        assert_eq!(queue_items, vec![(real_seq, "d".to_string())]);
        let rows = store.pending_inputs("s", Delivery::Queue).await.unwrap();
        assert_eq!(rows.len(), 1, "row must be durably admitted");
    }

    /// Delegates everything to an inner LibsqlStore EXCEPT `admit_input`,
    /// which always fails (steer_fire.rs `FailingAdmitStore` pattern).
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
        async fn admit_input(&self, _i: &SessionInput) -> Result<i64> {
            anyhow::bail!("admit failed")
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
        async fn complete_subagent_task(&self, id: &str, r: &str, ok: bool) -> Result<()> {
            self.0.complete_subagent_task(id, r, ok).await
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

    #[tokio::test]
    async fn actor_failure_path_flashes_and_removes_row() {
        let inner = Arc::new(LibsqlStore::open_memory().await.unwrap());
        let store: Arc<dyn Store> = Arc::new(FailingAdmitStore(inner));
        let (tx, mut done_rx) = spawn_admitter(store);
        let mut st = AdmitUiState::default();
        let mut queue_items = vec![(-100, "other".to_string())];
        let mut pending_images = vec![("img.png".to_string(), "alt".to_string())];
        assert!(submit(
            &tx,
            &mut st,
            &mut queue_items,
            &mut pending_images,
            mk_input("p"),
            "d".into()
        ));
        let done = done_rx.recv().await.unwrap();
        assert!(done.result.is_err());
        let flash = apply_done(&mut st, done, &mut queue_items, &mut pending_images);
        assert_eq!(
            flash,
            Some("⚠ queue submit failed — recover text with ↑ history")
        );
        assert_eq!(
            queue_items,
            vec![(-100, "other".to_string())],
            "temp row removed, others kept"
        );
        assert_eq!(
            pending_images,
            vec![("img.png".to_string(), "alt".to_string())]
        );
        assert!(st.inflight.is_empty());
    }
    #[tokio::test]
    async fn handle_queue_admits_raw_text_and_defers_skill() {
        let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
        store
            .create_session(&SessionMeta {
                id: "s".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let (tx, mut done_rx) = spawn_admitter(Arc::clone(&store) as Arc<dyn Store>);
        let mut st = AdmitUiState::default();
        let mut queue_items = vec![];
        let mut pending_images = vec![];
        handle_queue(
            "$alpha fix the bug",
            &tx,
            &mut st,
            &mut queue_items,
            &mut pending_images,
            "s",
        );

        // Queue-panel mirror shows what the user typed, token included.
        assert!(queue_items.iter().any(|(_, d)| d.contains("$alpha")));
        let done = done_rx.recv().await.unwrap();
        apply_done(&mut st, done, &mut queue_items, &mut pending_images);
        let rows = store.pending_inputs("s", Delivery::Queue).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].prompt, "$alpha fix the bug",
            "raw text queued verbatim — the runner resolves the token at consumption"
        );
        assert_eq!(rows[0].display_text.as_deref(), Some("$alpha fix the bug"));
        // No skill side effects at queue time: sessions.skill stays NULL.
        assert!(
            store
                .get_session("s")
                .await
                .unwrap()
                .and_then(|m| m.skill)
                .is_none(),
            "queueing must not persist a skill; that happens at consumption"
        );
    }

    #[tokio::test]
    async fn handle_queue_pure_skill_admits_token_not_trigger() {
        let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
        store
            .create_session(&SessionMeta {
                id: "s".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let (tx, mut done_rx) = spawn_admitter(Arc::clone(&store) as Arc<dyn Store>);
        let mut st = AdmitUiState::default();
        let mut queue_items = vec![];
        let mut pending_images = vec![];
        handle_queue(
            "$alpha",
            &tx,
            &mut st,
            &mut queue_items,
            &mut pending_images,
            "s",
        );

        let done = done_rx.recv().await.unwrap();
        apply_done(&mut st, done, &mut queue_items, &mut pending_images);
        let rows = store.pending_inputs("s", Delivery::Queue).await.unwrap();
        assert_eq!(
            rows[0].prompt, "$alpha",
            "pure-skill queue item admits the token verbatim; the runner's \
             record_compound injects SKILL_TRIGGER at consumption"
        );
        assert!(
            queue_items
                .iter()
                .all(|(_, d)| !d.contains("skill is now active")),
            "no synthetic trigger text is queued at submit time"
        );
    }
}
