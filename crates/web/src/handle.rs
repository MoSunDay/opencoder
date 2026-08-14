//! Web session runtime: per-session broadcast handles + background drain tasks.
//!
//! A `SessionHandle` owns a tokio `broadcast::Sender` of `SseEvt`. POST /prompt
//! admits an input to the store and ensures exactly one drain task is running;
//! the drain drives the real session runner, broadcasting events live. GET
//! /events replays persisted events after a cursor, then forwards the live
//! broadcast — so any process (or browser tab) sees a consistent stream.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Result;
use opencoder_core::Config;
use opencoder_llm::{ChatClient, ChatStream};
use opencoder_session::compaction;
use opencoder_session::plan_handoff;
use opencoder_session::tools::registry as build_registry;
use opencoder_session::{resume_and_replay as resume_session, run, SessionEvent};
use opencoder_store::{Delivery, EventKind, SessionInput, SessionPatch, Store};
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::cmd::DrainCmd;

/// Shared SSE envelope (re-exported from `opencoder-core` so the server and a
/// remote client agree on the wire shape).
pub use opencoder_core::SseEvt;

/// Build the wire SSE event + coarse DB kind from a session event.
pub fn sse_from_session_event(_session_id: &str, ev: &SessionEvent) -> (SseEvt, EventKind) {
    let ts = opencoder_core::message::now_ms();
    (
        SseEvt {
            kind: ev.sse_kind().to_string(),
            data: ev.sse_data(),
            ts,
            seq: None,
        },
        ev.coarse_kind(),
    )
}

/// Per-session runtime state shared across HTTP requests, SSE subscribers, and
/// the background drain task.
pub struct SessionHandle {
    pub tx: broadcast::Sender<SseEvt>,
    pub cancel: Mutex<CancellationToken>,
    pub overrides: Mutex<RuntimeOverrides>,
    pub draining: AtomicBool,
    /// Count of active SSE `/events` subscribers. Used by the events endpoint
    /// to evict a handle it created once the last subscriber disconnects (and
    /// no drain is running), so the handle map cannot grow without bound on a
    /// long-running server.
    pub subscribers: AtomicUsize,
    /// Sender for drain commands (compact, handoff, skill, config reload).
    pub cmd_tx: mpsc::UnboundedSender<DrainCmd>,
    /// Receiver for drain commands. `Option` so the drain task can `take()` it
    /// for exclusive access, then put it back. Panic-safe via `CmdRxGuard`.
    pub cmd_rx: std::sync::Mutex<Option<mpsc::UnboundedReceiver<DrainCmd>>>,
    pub child_turn_cancels: Arc<std::sync::Mutex<HashMap<String, opencoder_session::SharedCancel>>>,
    pub child_steer_gates:
        Arc<std::sync::Mutex<HashMap<String, Arc<opencoder_session::SubagentSteerGate>>>>,
    pub child_cancels: Arc<std::sync::Mutex<HashMap<String, CancellationToken>>>,
    /// Parent turn-level cancel token. When a steer is admitted while a drain
    /// is running, this fires so the current LLM turn / tool execution is
    /// interrupted immediately and the steer is absorbed at the next turn
    /// boundary — mirroring the TUI's `fire_steer_interrupt` → `SteerParent`
    /// path. Shared (Arc) with `SessionState` so `run_loop` can reset it
    /// after absorbing the cancelled turn.
    pub turn_cancel: opencoder_session::SharedCancel,
}

const BROADCAST_CAPACITY: usize = 256;

impl SessionHandle {
    pub fn new() -> Arc<Self> {
        let (tx, _rx) = broadcast::channel::<SseEvt>(BROADCAST_CAPACITY);
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<DrainCmd>();
        Arc::new(SessionHandle {
            tx,
            cancel: Mutex::new(CancellationToken::new()),
            overrides: Mutex::new(RuntimeOverrides::default()),
            draining: AtomicBool::new(false),
            subscribers: AtomicUsize::new(0),
            cmd_tx,
            cmd_rx: std::sync::Mutex::new(Some(cmd_rx)),
            child_turn_cancels: Arc::new(std::sync::Mutex::new(HashMap::new())),
            child_steer_gates: Arc::new(std::sync::Mutex::new(HashMap::new())),
            child_cancels: Arc::new(std::sync::Mutex::new(HashMap::new())),
            turn_cancel: Arc::new(std::sync::Mutex::new(CancellationToken::new())),
        })
    }
}

#[derive(Default)]
pub struct RuntimeOverrides {
    pub agent: Option<String>,
    pub model: Option<String>,
}

pub type HandleMap = Arc<Mutex<HashMap<String, Arc<SessionHandle>>>>;

pub fn new_handle_map() -> HandleMap {
    Arc::new(Mutex::new(HashMap::new()))
}

/// RAII guard that clears `draining` on drop (panic-safe).
struct DrainGuard {
    handle: Arc<SessionHandle>,
}

impl Drop for DrainGuard {
    fn drop(&mut self) {
        self.handle.draining.store(false, Ordering::SeqCst);
    }
}

/// RAII guard that restores `cmd_rx` into the handle on drop.
struct CmdRxGuard {
    handle: Arc<SessionHandle>,
    rx: Option<mpsc::UnboundedReceiver<DrainCmd>>,
}

impl Drop for CmdRxGuard {
    fn drop(&mut self) {
        if let Some(rx) = self.rx.take() {
            if let Ok(mut g) = self.handle.cmd_rx.lock() {
                *g = Some(rx);
            }
        }
    }
}

/// Stream wrapper that runs `on_drop` exactly once when the stream is dropped
/// (client disconnect OR natural end). The `/events` SSE endpoint uses it to
/// release its subscriber slot and evict a now-unused session handle, closing
/// the leak where `GET /events` created a handle that was never removed.
pub(crate) struct DropGuardStream<S> {
    inner: std::pin::Pin<Box<S>>,
    on_drop: Option<Box<dyn FnOnce() + Send + Sync>>,
}

impl<S: futures::Stream> DropGuardStream<S> {
    /// Wrap `stream` so `on_drop` runs once when the stream is dropped.
    pub(crate) fn new(
        stream: S,
        on_drop: impl FnOnce() + Send + Sync + 'static,
    ) -> Self {
        DropGuardStream {
            inner: Box::pin(stream),
            on_drop: Some(Box::new(on_drop)),
        }
    }
}

impl<S: futures::Stream> futures::Stream for DropGuardStream<S> {
    type Item = S::Item;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

impl<S> Drop for DropGuardStream<S> {
    fn drop(&mut self) {
        if let Some(f) = self.on_drop.take() {
            f();
        }
    }
}

/// Decrement a handle's subscriber slot without ever underflowing, returning
/// the value observed before the decrement. The map lookup is keyed by session
/// id, so a release aimed at an OLD instance can land on a freshly created
/// same-id handle whose counter is 0; a blind `fetch_sub` would wrap it to
/// `usize::MAX`, permanently disabling last-subscriber eviction for that
/// handle. `Err(current)` from `fetch_update` means the counter was already 0
/// (f returned `None`) — report 0, never a wrapped value.
fn release_subscriber_slot(h: &SessionHandle) -> usize {
    match h.subscribers.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
        if v == 0 { None } else { Some(v - 1) }
    }) {
        Ok(prev) => prev,
        Err(current) => current,
    }
}

/// Decrement an events subscriber and, when THIS request *created* the handle,
/// evict it once the last subscriber is gone and no drain is running. Spawned
/// (async work can't run in `Drop`); all subscribe+increment happen under the
/// same `HandleMap` lock this holds, so the "last subscriber" check is
/// authoritative w.r.t. concurrent subscribers — evicting only when nobody else
/// is listening means dropping the broadcast Sender breaks no live Receiver.
#[allow(unused_variables)]
pub(crate) fn release_events_subscriber(handles: HandleMap, id: String, created: bool) {
    if let Ok(rt) = tokio::runtime::Handle::try_current() {
        rt.spawn(async move {
            let mut evict = false;
            {
                let mut map = handles.lock().await;
                if let Some(h) = map.get(&id) {
                    let prev = release_subscriber_slot(h);
                    if prev == 1 && !h.draining.load(Ordering::SeqCst) {
                        map.remove(&id);
                        evict = true;
                    }
                }
            }
            if evict {
                opencoder_session::mcp::cleanup(&id).await;
            }
        });
    } else {
        let mut evict = false;
        {
            let mut map = handles.blocking_lock();
            if let Some(h) = map.get(&id) {
                let prev = release_subscriber_slot(h);
                if prev == 1 && !h.draining.load(Ordering::SeqCst) {
                    map.remove(&id);
                    evict = true;
                }
            }
        }
        if evict {
            // No async runtime available — spawn a detached thread.
            let id_clone = id.clone();
            std::thread::spawn(move || {
                // best-effort: block on a temporary runtime
                let rt = tokio::runtime::Runtime::new().ok();
                if let Some(rt) = rt {
                    rt.block_on(async { opencoder_session::mcp::cleanup(&id_clone).await });
                }
            });
        }
    }
}

/// Admit a prompt durably, then ensure exactly one drain task is running.
#[allow(clippy::too_many_arguments)]
pub async fn admit_and_drain(
    handles: HandleMap,
    store: Arc<dyn Store>,
    session_id: &str,
    prompt: String,
    images: Vec<String>,
    delivery: Delivery,
    client: Arc<dyn ChatStream>,
    workdir: std::path::PathBuf,
    config: Config,
) -> Result<i64> {
    let input = SessionInput {
        seq: None,
        id: uuid::Uuid::new_v4().to_string(),
        session_id: session_id.to_string(),
        delivery,
        prompt,
        images,
        display_text: None,
        admitted_seq: 0,
        promoted_seq: None,
    };
    let seq = store.admit_input(&input).await?;

    let handle = {
        let mut map = handles.lock().await;
        map.entry(session_id.to_string())
            .or_insert_with(SessionHandle::new)
            .clone()
    };

    let started_new_drain = !handle.draining.swap(true, Ordering::SeqCst);
    if started_new_drain {
        let token = CancellationToken::new();
        *handle.cancel.lock().await = token.clone();
        // Reset the turn-level token so the new drain starts clean (a
        // previous drain may have been turn-cancelled without resetting).
        if let Ok(mut g) = handle.turn_cancel.lock() {
            *g = CancellationToken::new();
        }
        let handles_clone = handles.clone();
        let store_clone = store.clone();
        let sid = session_id.to_string();
        let cfg = config.clone();
        let client_clone = client.clone();
        let wd = workdir.clone();
        let handle_clone = handle.clone();
        tokio::spawn(async move {
            drain_to_completion(
                handles_clone,
                store_clone,
                &sid,
                client_clone,
                wd,
                cfg,
                handle_clone,
            )
            .await;
        });
    } else {
        // Steer admitted while a drain is running: fire the parent's
        // turn-level cancel so the current LLM turn (or tool execution) is
        // interrupted immediately and the steer is absorbed at the next turn
        // boundary. Mirrors the TUI's SteerParent path. Queue inputs are
        // consumed at idle — no interrupt needed.
        if delivery == Delivery::Steer {
            opencoder_session::fire_turn_cancel(&handle.turn_cancel);
            opencoder_session::fire_child_cancels(&handle.child_cancels);
        }
        let handles_w = handles.clone();
        let store_w = store.clone();
        let sid_w = session_id.to_string();
        let cfg_w = config.clone();
        let client_w = client.clone();
        let wd_w = workdir.clone();
        let handle_w = handle.clone();
        tokio::spawn(async move {
            // Poll until the in-flight drain finishes. Real thinking phases
            // last 10-60s+ (and long tool chains far longer), so the cap must
            // comfortably exceed the longest legitimate drain; the previous
            // 5s cap abandoned the drain mid-thinking and prevented the
            // defense-in-depth restart below from ever firing. 12_000 * 50ms
            // = 10 min; the atomic load is ~free and the swap guard at the
            // bottom prevents duplicate drains.
            for _ in 0..12_000 {
                if !handle_w.draining.load(Ordering::SeqCst) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            if handle_w.draining.load(Ordering::SeqCst) {
                return;
            }
            // Defense-in-depth: a drain can exit with steers still pending
            // (e.g. a steer stranded by a residual idle-boundary window, or a
            // crashed drain). Restart the drain if EITHER a queued or a
            // steered input is waiting so neither delivery channel strands.
            let pending_q = store_w
                .pending_inputs(&sid_w, opencoder_store::Delivery::Queue)
                .await
                .unwrap_or_default();
            let pending_s = store_w
                .pending_inputs(&sid_w, opencoder_store::Delivery::Steer)
                .await
                .unwrap_or_default();
            if pending_q.is_empty() && pending_s.is_empty() {
                return;
            }
            if !handle_w.draining.swap(true, Ordering::SeqCst) {
                let token = CancellationToken::new();
                *handle_w.cancel.lock().await = token.clone();
                if let Ok(mut g) = handle_w.turn_cancel.lock() {
                    *g = CancellationToken::new();
                }
                drain_to_completion(handles_w, store_w, &sid_w, client_w, wd_w, cfg_w, handle_w)
                    .await;
            }
        });
    }
    if started_new_drain {
        let _ = opencoder_session::fire_child_cancels(&handle.child_cancels);
    }
    Ok(seq)
}

/// Ensure exactly one drain task is running WITHOUT admitting a prompt.
/// Used by command endpoints (POST /compact, /handoff, etc.).
pub async fn ensure_drain(
    handles: HandleMap,
    store: Arc<dyn Store>,
    session_id: &str,
    client: Arc<dyn ChatStream>,
    workdir: std::path::PathBuf,
    config: Config,
) {
    let handle = {
        let mut map = handles.lock().await;
        map.entry(session_id.to_string())
            .or_insert_with(SessionHandle::new)
            .clone()
    };
    if !handle.draining.swap(true, Ordering::SeqCst) {
        let token = CancellationToken::new();
        *handle.cancel.lock().await = token.clone();
        if let Ok(mut g) = handle.turn_cancel.lock() {
            *g = CancellationToken::new();
        }
        let handles_clone = handles.clone();
        let store_clone = store.clone();
        let sid = session_id.to_string();
        let cfg = config.clone();
        let client_clone = client.clone();
        let wd = workdir.clone();
        let handle_clone = handle.clone();
        tokio::spawn(async move {
            drain_to_completion(
                handles_clone,
                store_clone,
                &sid,
                client_clone,
                wd,
                cfg,
                handle_clone,
            )
            .await;
        });
    }
}

/// Send a drain command to the session's handle.
pub async fn send_cmd(handles: &HandleMap, session_id: &str, cmd: DrainCmd) -> bool {
    let map = handles.lock().await;
    if let Some(h) = map.get(session_id) {
        if let Err(e) = h.cmd_tx.send(cmd) {
            warn!(error = %e, session_id = %session_id, "send_cmd: drain command not delivered (drain exited?)");
        }
        true
    } else {
        false
    }
}

/// Apply a single drain command to the live `&mut SessionState`.
async fn apply_drain_cmd(
    session: &mut opencoder_session::SessionState,
    cmd: DrainCmd,
    tx: &broadcast::Sender<SseEvt>,
    sink: &opencoder_session::EventSink,
    sid: &str,
    workdir: &std::path::Path,
) {
    // Mirror the main `run` callback: broadcast to live SSE subscribers AND
    // persist the event to the `session_events` table via the sink. Without
    // the `sink.push`, drain-command events (Compaction, Done,
    // TranscriptReset, PlanHandoff, …) would never reach disk, so an SSE
    // reconnect replay (`?after=<seq>`) would silently miss them.
    let mut broadcast = |ev: SessionEvent| {
        let (sse, _) = sse_from_session_event(sid, &ev);
        let _ = tx.send(sse);
        let _ = sink.push(&ev);
    };
    match cmd {
        DrainCmd::Compact => {
            let registry = build_registry();
            match compaction::compact(session, &registry, &mut broadcast).await {
                Ok(_) => broadcast(SessionEvent::Done),
                Err(e) => broadcast(SessionEvent::Error(format!("compact: {e:#}"))),
            }
        }
        DrainCmd::Handoff { extra } => {
            if let Some(plan) = plan_handoff::handoff(session, &extra) {
                if let Some(store) = &session.store {
                    let _ = store
                        .update_session(
                            &session.id,
                            &SessionPatch {
                                agent: Some("act".into()),
                                handoff_seq: session.handoff_seq,
                                handoff_plan: session.handoff_plan.clone(),
                                updated_at: Some(opencoder_core::message::now_ms()),
                                ..Default::default()
                            },
                        )
                        .await;
                }
                broadcast(SessionEvent::TranscriptReset(session.messages.clone()));
                broadcast(SessionEvent::PlanHandoff(plan));
                broadcast(SessionEvent::Done);
            } else {
                broadcast(SessionEvent::Error("no plan to hand off".into()));
            }
        }
        DrainCmd::SetSkill(body) => {
            session.set_skill(body);
            broadcast(SessionEvent::Done);
        }
        DrainCmd::ReloadConfig => match Config::load(workdir) {
            Ok(new_cfg) => {
                match new_cfg.resolve_endpoint() {
                    Ok(ep) => match ChatClient::new_with_read_timeout(
                        &ep.base_url,
                        &ep.api_key,
                        &ep.headers,
                        new_cfg.stream_idle_timeout(),
                        new_cfg.network.proxy.as_deref(),
                    ) {
                        Ok(c) => {
                            session.apply_config_reload(new_cfg, Arc::new(c) as Arc<dyn ChatStream>)
                        }
                        Err(_) => session.apply_config_reload_keep_client(new_cfg),
                    },
                    Err(_) => session.apply_config_reload_keep_client(new_cfg),
                }
                // Sync MCP connections with the reloaded config.
                let desired: Vec<_> = session
                    .config
                    .enabled_mcp_servers()
                    .into_iter()
                    .map(|(n, c)| (n, c.clone()))
                    .collect();
                opencoder_session::mcp::pool::sync(&session.id, &desired).await;
                broadcast(SessionEvent::Done);
            }
            Err(e) => broadcast(SessionEvent::Error(format!("reload config: {e:#}"))),
        },
    }
}

/// Drain all pending commands from the receiver and apply them in order.
async fn process_drain_cmds(
    session: &mut opencoder_session::SessionState,
    rx_guard: &mut CmdRxGuard,
    tx: &broadcast::Sender<SseEvt>,
    sink: &opencoder_session::EventSink,
    sid: &str,
    workdir: &std::path::Path,
) {
    if let Some(rx) = rx_guard.rx.as_mut() {
        while let Ok(cmd) = rx.try_recv() {
            apply_drain_cmd(session, cmd, tx, sink, sid, workdir).await;
        }
    }
}

/// Drive the session runner to completion, broadcasting events.
async fn drain_to_completion(
    handles: HandleMap,
    store: Arc<dyn Store>,
    session_id: &str,
    client: Arc<dyn ChatStream>,
    workdir: std::path::PathBuf,
    mut config: Config,
    handle: Arc<SessionHandle>,
) {
    let guard = DrainGuard {
        handle: handle.clone(),
    };
    let mut rx_guard = CmdRxGuard {
        handle: handle.clone(),
        rx: handle.cmd_rx.lock().map(|mut g| g.take()).ok().flatten(),
    };

    {
        let ov = handle.overrides.lock().await;
        if let Some(a) = &ov.agent {
            config.agent.default = a.clone();
        }
        if let Some(m) = &ov.model {
            config.model = m.clone();
        }
    }

    let cancel_token = handle.cancel.lock().await.clone();
    let mut session = match resume_session(
        store.clone(),
        session_id,
        config.clone(),
        client.clone(),
        workdir.clone(),
        Some(cancel_token),
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            warn!(session_id, error = %e, "drain: cannot resume (session row missing?)");
            let mut map = handles.lock().await;
            // Only reclaim the map entry when nobody is listening. Live SSE
            // subscribers still hold THIS handle's broadcast receiver: removing
            // the entry would orphan them (a later prompt creates a NEW
            // handle/tx they never receive, and their eventual
            // `release_events_subscriber` would decrement that fresh instance's
            // counter — underflow). The check runs under the map lock, which is
            // the same lock every subscribe/increment takes, so a zero count is
            // authoritative. With subscribers attached, keep the entry: the
            // normal eviction path (last subscriber leaves while idle) reclaims
            // it later. Also only remove when the entry is still THIS instance —
            // it may have been deleted + recreated meanwhile (e.g. DELETE).
            let still_current = map
                .get(session_id)
                .is_some_and(|h| Arc::ptr_eq(h, &handle));
            let live = handle.subscribers.load(Ordering::SeqCst) > 0
                || handle.tx.receiver_count() > 0;
            if live {
                warn!(
                    session_id,
                    subscribers = handle.subscribers.load(Ordering::SeqCst),
                    "drain: resume failed but SSE subscribers remain; keeping handle"
                );
            } else if still_current {
                map.remove(session_id);
            }
            return;
        }
    };
    session.cancel = Some(handle.cancel.lock().await.clone());
    session.child_turn_cancels = handle.child_turn_cancels.clone();
    session.child_steer_gates = handle.child_steer_gates.clone();
    session.child_cancels = handle.child_cancels.clone();
    session.turn_cancel = Some(handle.turn_cancel.clone());

    let tx = handle.tx.clone();
    let sid = session_id.to_string();
    let (sink, flusher) =
        opencoder_session::spawn_event_flusher(Some(store.clone()), session_id.to_string());
    let result = run(&mut session, String::new(), |ev| {
        let (sse, _kind) = sse_from_session_event(&sid, &ev);
        let _ = tx.send(sse);
        let _ = sink.push(&ev);
    })
    .await;

    // After run completes, process any queued drain commands.
    process_drain_cmds(&mut session, &mut rx_guard, &tx, &sink, &sid, &workdir).await;

    drop(sink);
    drop(guard);
    if let Err(e) = flusher.await {
        warn!(session_id, error = %e, "final event flush failed");
    }
    drop(rx_guard);
    if let Err(e) = result {
        warn!(session_id, error = %e, "drain ended with error");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[tokio::test]
    async fn release_subscriber_evicts_creator_handle_when_last_and_idle() {
        let handles = new_handle_map();
        let id = "sess-evict".to_string();
        let h = SessionHandle::new();
        h.subscribers.store(1, Ordering::SeqCst);
        h.draining.store(false, Ordering::SeqCst);
        handles.lock().await.insert(id.clone(), h);

        release_events_subscriber(handles.clone(), id.clone(), true);

        // The eviction runs in a spawned task; poll until it settles.
        for _ in 0..200 {
            if !handles.lock().await.contains_key(&id) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        assert!(
            !handles.lock().await.contains_key(&id),
            "creator handle should be evicted when last subscriber leaves and idle"
        );
    }

    #[tokio::test]
    async fn release_subscriber_keeps_handle_while_draining() {
        let handles = new_handle_map();
        let id = "sess-drain".to_string();
        let h = SessionHandle::new();
        h.subscribers.store(1, Ordering::SeqCst);
        h.draining.store(true, Ordering::SeqCst);
        handles.lock().await.insert(id.clone(), h);

        release_events_subscriber(handles.clone(), id.clone(), true);
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert!(
            handles.lock().await.contains_key(&id),
            "handle must survive while a drain is running"
        );
    }

    #[tokio::test]
    async fn release_subscriber_keeps_handle_while_others_remain() {
        let handles = new_handle_map();
        let id = "sess-guest".to_string();
        let h = SessionHandle::new();
        // Two subscribers; a single non-creator release (prev == 2) is NOT the
        // last subscriber, so the handle must survive.
        h.subscribers.store(2, Ordering::SeqCst);
        h.draining.store(false, Ordering::SeqCst);
        handles.lock().await.insert(id.clone(), h);

        release_events_subscriber(handles.clone(), id.clone(), false);
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert!(
            handles.lock().await.contains_key(&id),
            "handle must survive when a non-creator leaves but another subscriber remains"
        );
    }

    // Bug #4: eviction must not depend on the `created` flag. If the creator
    // disconnects first while a second (non-creator) subscriber remains, that
    // second subscriber must still evict the handle when it becomes the last
    // one leaving. The old `created &&` condition skipped eviction for the
    // non-creator, leaking the handle forever.
    #[tokio::test]
    async fn session_handle_evicted_when_creator_leaves_first() {
        let handles = new_handle_map();
        let id = "test-session".to_string();

        // Simulate subscriber A creating the handle (creator).
        {
            let mut map = handles.lock().await;
            let handle = map
                .entry(id.clone())
                .or_insert_with(SessionHandle::new);
            handle
                .subscribers
                .fetch_add(1, Ordering::SeqCst);
        }

        // Simulate subscriber B joining (non-creator).
        {
            let mut map = handles.lock().await;
            let handle = map
                .entry(id.clone())
                .or_insert_with(SessionHandle::new);
            handle
                .subscribers
                .fetch_add(1, Ordering::SeqCst);
        }

        // Creator A leaves first (created=true, prev=2 → not the last, kept).
        release_events_subscriber(handles.clone(), id.clone(), true);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        {
            let map = handles.lock().await;
            assert!(
                map.contains_key(&id),
                "handle should survive when creator leaves but another subscriber remains"
            );
        }

        // Subscriber B leaves last (created=false, prev=1 → must be evicted).
        release_events_subscriber(handles.clone(), id.clone(), false);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        {
            let map = handles.lock().await;
            assert!(
                !map.contains_key(&id),
                "handle should be evicted when last subscriber leaves, even if not the creator"
            );
        }
    }

    // Bug: `release_events_subscriber` looks the handle up by session id, so a
    // release aimed at an OLD (already-removed) instance can land on a freshly
    // created same-id handle whose counter is 0. A blind `fetch_sub` wraps to
    // `usize::MAX`, corrupting the count and disabling last-subscriber
    // eviction forever. The decrement must saturate at 0.
    #[tokio::test]
    async fn release_subscriber_does_not_underflow_fresh_instance() {
        let handles = new_handle_map();
        let id = "sess-underflow".to_string();
        let h = SessionHandle::new();
        // Fresh instance: no subscriber ever attached (count 0), not draining.
        h.subscribers.store(0, Ordering::SeqCst);
        h.draining.store(false, Ordering::SeqCst);
        handles.lock().await.insert(id.clone(), h.clone());

        // A stale release for the old same-id instance fires on this handle.
        release_events_subscriber(handles.clone(), id.clone(), true);

        // The release runs in a spawned task; poll until it settles (counter
        // either changed or the sleep guarantees the task ran).
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(
            h.subscribers.load(Ordering::SeqCst),
            0,
            "subscriber counter must saturate at 0, not wrap to usize::MAX"
        );
        assert!(
            handles.lock().await.contains_key(&id),
            "a zero-count release must not evict the handle"
        );
    }
}
