//! TUI-internal subtask event hub. Watcher deliveries arrive on a channel
//! owned here — NOT the session worker channel, which dies on every `/task`
//! switch — while subtask writebacks must keep flowing to the owning parent.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use opencoder_core::Config;
use opencoder_store::Store;
use tokio::sync::mpsc;

use opencoder_core::harness::ServerCapability;

use crate::chat::ChatView;
use crate::remote::client::ServerClient;
use crate::remote::subtasks::DispatchOutcome;
use crate::session_ui::SessionUiState;
use crate::worker::UiEvent;

/// Watcher -> main-loop channel capacity (drained every loop tick).
const EVENT_CHANNEL_CAPACITY: usize = 64;

/// Hub state: plain data + free-standing methods — no actor of its own
/// (watchers spawned by [`SubtaskUi::maybe_spawn_watcher`] report here).
pub(crate) struct SubtaskUi {
    /// Parent session the TUI descended from into a subtask child session.
    pub main_task_id: Option<String>,
    /// Execution ids ever handed to a watcher (terminal transitions are
    /// one-way, so the set only grows; prevents duplicate pollers).
    watched: HashSet<String>,
    /// In-flight `@capability question` one-shot dispatch.
    pub dispatch: Option<tokio::sync::oneshot::Receiver<anyhow::Result<DispatchOutcome>>>,
    /// Capability catalog backing the `@capability` mention picker (kept
    /// warm at startup and after every dispatch; errors surface lazily).
    pub caps: Vec<ServerCapability>,
    /// In-flight capability-catalog fetch (skipped while one is pending).
    caps_refresh: Option<tokio::sync::oneshot::Receiver<anyhow::Result<Vec<ServerCapability>>>>,

    /// Ids locally settled from a terminal cancel response: the watcher
    /// drops them and late deliveries are ignored (a dead 404 poller can
    /// never overwrite the settled status with `error`).
    settled: Arc<std::sync::Mutex<HashSet<String>>>,
    /// Watcher -> main-loop events (dedicated: workers die on `/task` switch).
    events: (mpsc::Sender<UiEvent>, mpsc::Receiver<UiEvent>),
}

impl SubtaskUi {
    pub(crate) fn new() -> Self {
        let events = mpsc::channel(EVENT_CHANNEL_CAPACITY);
        SubtaskUi {
            main_task_id: None,
            watched: HashSet::new(),
            dispatch: None,
            caps: Vec::new(),
            caps_refresh: None,
            settled: Arc::new(std::sync::Mutex::new(HashSet::new())),
            events,
        }
    }

    /// Cloneable sender half handed to spawned watchers.
    pub(crate) fn ui_sender(&self) -> mpsc::Sender<UiEvent> {
        self.events.0.clone()
    }

    /// Settle a session switch: success adopts `parent_after_switch` as
    /// `main_task_id` (clearing it for a plain switch); failure keeps the
    /// current descent pointer and drops the staged intent.
    pub(crate) fn settle_switch(&mut self, switched: bool, parent_after_switch: Option<String>) {
        if switched {
            self.main_task_id = parent_after_switch;
        }
    }

    /// Spawn a watcher per still-pending registry entry lacking a poller;
    /// client-config errors skip silently (the next dispatch retries).
    pub(crate) async fn maybe_spawn_watcher(
        &mut self,
        config: &Config,
        store: &Arc<dyn Store>,
        session_id: &str,
    ) {
        let registry = crate::remote::subtasks::load_registry(store, session_id)
            .await
            .unwrap_or_default();
        let pending = crate::subtask_registry::pending_ids(&registry);
        let targets: Vec<crate::subtask_watcher::WatchTarget> = pending
            .into_iter()
            .filter(|id| !self.watched.contains(id))
            .map(|execution_id| crate::subtask_watcher::WatchTarget {
                session: session_id.to_string(),
                execution_id,
            })
            .collect();
        if targets.is_empty() {
            return;
        }
        let Ok(client) = crate::remote::client::ServerClient::configured(config) else {
            return;
        };
        self.watched
            .extend(targets.iter().map(|t| t.execution_id.clone()));
        crate::subtask_watcher::spawn_with_settled(
            client,
            self.ui_sender(),
            targets,
            self.settled.clone(),
        );
    }

    /// Remember an id settled locally (terminal cancel response); the
    /// watcher stops polling it and the event drain ignores late updates.
    pub(crate) fn mark_locally_settled(&self, id: &str) {
        self.settled
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.to_string());
    }

    /// Whether `id` was settled locally by the cancel-response path.
    pub(crate) fn is_locally_settled(&self, id: &str) -> bool {
        self.settled
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(id)
    }

    /// Apply a locally terminal cancel status to the owning registry. The id
    /// is marked settled ONLY after the registry write succeeds, so a failed
    /// save leaves the watcher as the fallback. Token delta is always 0 (the
    /// cancel response carries no usage) and summary/tokens are preserved
    /// except the `cancelled` summary clear in [`settle_status_only`].
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn settle_local(
        &self,
        store: &Arc<dyn Store>,
        session_states: &mut HashMap<String, SessionUiState>,
        current_session: &str,
        chat: &mut ChatView,
        ev_session: &str,
        ev_id: &str,
        status: &str,
    ) -> bool {
        let registry = crate::remote::subtasks::load_registry(store, ev_session)
            .await
            .unwrap_or_default();
        let refs = crate::subtask_registry::settle_status_only(&registry, ev_id, status);
        // Unknown id or an idempotent re-settle: nothing to persist.
        if refs == registry {
            return false;
        }
        if crate::remote::subtasks::save_registry(store, ev_session, &refs)
            .await
            .is_err()
        {
            return false;
        }
        self.mark_locally_settled(ev_id);
        let done = crate::subtask_registry::done_count(&refs);
        let total = crate::subtask_registry::total_count(&refs);
        if ev_session == current_session {
            chat.subtask_done = done;
            chat.subtask_total = total;
        } else if let Some(state) = session_states.get_mut(ev_session) {
            state.chat.subtask_done = done;
            state.chat.subtask_total = total;
        }
        true
    }

    /// Non-blocking poll of an in-flight dispatch (same pattern as
    /// `remote::selection::poll`): `None` while pending, else the result
    /// (slot cleared either way).
    pub(crate) fn poll_dispatch(&mut self) -> Option<anyhow::Result<DispatchOutcome>> {
        let receiver = self.dispatch.as_mut()?;
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => return None,
            Err(error) => Err(error.into()),
        };
        self.dispatch = None;
        Some(result)
    }

    /// Drain queued watcher events (raw `SubtaskUpdate`s; strays dropped).
    pub(crate) fn drain_updates(&mut self) -> Vec<UiEvent> {
        let mut out = Vec::new();
        while let Ok(event) = self.events.1.try_recv() {
            if matches!(event, UiEvent::SubtaskUpdate { .. }) {
                out.push(event);
            }
        }
        out
    }

    /// Kick off a capability-catalog fetch (same one-shot pattern as
    /// [`Self::poll_dispatch`]). No-op when the remote side is disabled
    /// (`configured` would fail anyway) or a fetch is already in flight —
    /// the @ mention path flashes its own failures, so errors stay silent.
    pub(crate) fn refresh_caps(&mut self, config: &Config) {
        if self.caps_refresh.is_some() {
            return;
        }
        let Ok(client) = ServerClient::configured(config) else {
            return;
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.caps_refresh = Some(rx);
        tokio::spawn(async move {
            let _ = tx.send(client.capabilities().await);
        });
    }

    /// Non-blocking poll of an in-flight capability fetch: `None` while
    /// pending, else the result (slot cleared either way).
    pub(crate) fn poll_caps(&mut self) -> Option<anyhow::Result<Vec<ServerCapability>>> {
        let receiver = self.caps_refresh.as_mut()?;
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => return None,
            Err(error) => Err(error.into()),
        };
        self.caps_refresh = None;
        Some(result)
    }
}

/// Fold one watcher delivery into the owning session's registry and the UI
/// surfaces that mirror it. Returns `Some((done, total, delta))` when the
/// registry changed (callers flash terminal transitions); `None` for unknown
/// ids, idempotent redeliveries and missing runtimes (nothing was written).
/// Live session: `chat` counters recompute, `delta` folds into `[tok cost]`.
/// Dormant snapshot: ONLY `[sub x/x]` recompute — the switch-restore floor
/// already covers dormant token folds (adding `delta` would double-count).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn apply_update(
    store: &Arc<dyn Store>,
    session_states: &mut HashMap<String, SessionUiState>,
    current_session: &str,
    chat: &mut ChatView,
    ev_session: &str,
    ev_id: &str,
    ev_status: &str,
    ev_summary: Option<&str>,
    ev_tokens: u64,
) -> Option<(u32, u32, u64)> {
    let registry = crate::remote::subtasks::load_registry(store, ev_session)
        .await
        .unwrap_or_default();
    let (refs, delta) =
        crate::subtask_registry::apply_update(&registry, ev_id, ev_status, ev_summary, ev_tokens);
    // Unknown id, idempotent redelivery, or no runtime to own it.
    if refs == registry && delta == 0 {
        return None;
    }
    // Registry vanished underneath us (session deleted): treat as no-op.
    if crate::remote::subtasks::save_registry(store, ev_session, &refs)
        .await
        .is_err()
    {
        return None;
    }
    let done = crate::subtask_registry::done_count(&refs);
    let total = crate::subtask_registry::total_count(&refs);
    if ev_session == current_session {
        chat.subtask_done = done;
        chat.subtask_total = total;
        chat.tokens_total = chat.tokens_total.saturating_add(delta);
    } else if let Some(state) = session_states.get_mut(ev_session) {
        state.chat.subtask_done = done;
        state.chat.subtask_total = total;
    }
    Some((done, total, delta))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::subtasks::load_registry;
    use crate::subtask_registry::find;
    use opencoder_core::harness::{HarnessRuntime, SubtaskRef};
    use opencoder_store::{SessionMeta, TursoStore};

    fn entry(id: &str, status: &str, tokens: u64) -> SubtaskRef {
        SubtaskRef {
            execution_id: id.into(),
            kind: "agent".into(),
            capability_id: "ops_x".into(),
            question: format!("question {id}"),
            status: status.into(),
            result_summary: None,
            tokens,
            created_at: 0,
        }
    }

    /// TursoStore with a session whose harness runtime owns `refs`.
    async fn store_with_registry(sid: &str, refs: Vec<SubtaskRef>) -> Arc<dyn Store> {
        let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
        store
            .create_session(&SessionMeta {
                id: sid.into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let runtime = HarnessRuntime {
            subtasks: refs,
            ..Default::default()
        };
        store.set_harness_runtime(sid, &runtime).await.unwrap();
        store
    }

    #[tokio::test]
    async fn terminal_update_on_current_session_folds_delta_and_counters() {
        let store = store_with_registry("parent", vec![entry("agent-a", "running", 40)]).await;
        let mut states = HashMap::new();
        let mut chat = ChatView {
            agent: "act".into(),
            ..Default::default()
        };
        chat.tokens_total = 500;
        let outcome = apply_update(
            &store,
            &mut states,
            "parent",
            &mut chat,
            "parent",
            "agent-a",
            "done",
            Some("answer"),
            90,
        )
        .await;
        assert_eq!(outcome, Some((1, 1, 50)));
        assert_eq!(chat.subtask_done, 1);
        assert_eq!(chat.subtask_total, 1);
        assert_eq!(chat.tokens_total, 550, "delta folds into [tok cost]");
        let saved = load_registry(&store, "parent").await.unwrap();
        assert_eq!(find(&saved, "agent-a").unwrap().status, "done");
    }

    /// A terminal cancel status settles the registry in place: only the
    /// status changes (summary cleared for `cancelled`, tokens kept), the
    /// counters recompute, no token delta folds, and the id is marked
    /// locally settled for the watcher/event-drain guards.
    #[tokio::test]
    async fn settle_local_updates_registry_and_counters() {
        let mut parent = entry("agent-a", "running", 40);
        parent.result_summary = Some("partial".into());
        let store = store_with_registry("parent", vec![parent]).await;
        let mut states = HashMap::new();
        let mut chat = ChatView {
            tokens_total: 100,
            ..Default::default()
        };
        let ui = SubtaskUi::new();
        let settled = ui
            .settle_local(
                &store,
                &mut states,
                "parent",
                &mut chat,
                "parent",
                "agent-a",
                "cancelled",
            )
            .await;
        assert!(settled, "changed registry settles locally");
        let saved = load_registry(&store, "parent").await.unwrap();
        let item = find(&saved, "agent-a").expect("registry kept the entry");
        assert_eq!(item.status, "cancelled");
        assert_eq!(item.result_summary, None, "cancelled clears the summary");
        assert_eq!(item.tokens, 40, "tokens are preserved");
        assert_eq!(chat.subtask_done, 1);
        assert_eq!(chat.subtask_total, 1);
        assert_eq!(chat.tokens_total, 100, "no token delta on a cancel");
        assert!(ui.is_locally_settled("agent-a"));
    }

    #[tokio::test]
    async fn repeated_delivery_is_a_noop() {
        let store = store_with_registry("parent", vec![entry("agent-a", "done", 100)]).await;
        let mut states = HashMap::new();
        let mut chat = ChatView::default();
        let outcome = apply_update(
            &store,
            &mut states,
            "parent",
            &mut chat,
            "parent",
            "agent-a",
            "done",
            Some("same"),
            100,
        )
        .await;
        assert_eq!(outcome, None, "identical redelivery changes nothing");
        assert_eq!(chat.subtask_done, 0, "live chat untouched");
        assert_eq!(chat.tokens_total, 0);
    }

    #[tokio::test]
    async fn dormant_session_updates_snapshot_counters_only() {
        let store = store_with_registry("other", vec![entry("agent-a", "running", 10)]).await;
        let mut snapshot = SessionUiState::new("act".into(), 0);
        snapshot.chat.tokens_total = 300;
        snapshot.subtask_tokens_seen = 10;
        let mut states = HashMap::from([("other".to_string(), snapshot)]);
        let mut chat = ChatView::default();
        let outcome = apply_update(
            &store,
            &mut states,
            "live",
            &mut chat,
            "other",
            "agent-a",
            "done",
            Some("answer"),
            60,
        )
        .await;
        assert_eq!(outcome, Some((1, 1, 50)));
        let snap = &states["other"];
        assert_eq!(snap.chat.subtask_done, 1, "snapshot counters refresh");
        assert_eq!(snap.chat.subtask_total, 1);
        assert_eq!(
            snap.chat.tokens_total, 300,
            "delta must NOT fold into a dormant snapshot (restore floor covers it)"
        );
        assert_eq!(
            chat.subtask_done, 0,
            "live chat untouched by other sessions"
        );
    }

    #[tokio::test]
    async fn missing_runtime_is_a_noop() {
        let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
        let mut states = HashMap::new();
        let mut chat = ChatView::default();
        let outcome = apply_update(
            &store,
            &mut states,
            "ghost",
            &mut chat,
            "ghost",
            "agent-a",
            "done",
            None,
            10,
        )
        .await;
        assert_eq!(outcome, None);
    }

    /// With the remote side disabled there is no client to build, so the
    /// catalog fetch must stay idle (and the in-flight guard holds).
    #[tokio::test]
    async fn refresh_caps_is_a_noop_with_remote_disabled() {
        let mut ui = SubtaskUi::new();
        ui.refresh_caps(&Config::default());
        assert!(ui.caps_refresh.is_none(), "no request was armed");
        assert!(ui.poll_caps().is_none());
    }

    /// A successful switch lands the staged `/subtasks` descent parent.
    #[test]
    fn switch_landing_adopts_staged_parent() {
        let mut ui = SubtaskUi::new();
        ui.main_task_id = Some("old".into());
        ui.settle_switch(true, Some("parent".into()));
        assert_eq!(ui.main_task_id.as_deref(), Some("parent"));
    }

    /// A failed switch keeps the current descent pointer and drops the intent.
    #[test]
    fn switch_error_drops_staged_arm_and_keeps_current_descent() {
        let mut ui = SubtaskUi::new();
        ui.main_task_id = Some("old".into());
        ui.settle_switch(false, Some("parent".into()));
        assert_eq!(ui.main_task_id.as_deref(), Some("old"));
    }
}
