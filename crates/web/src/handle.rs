//! Web session runtime: per-session broadcast handles + background drain tasks.
//!
//! A `SessionHandle` owns a tokio `broadcast::Sender` of `SseEvt`. POST /prompt
//! admits an input to the store and ensures exactly one drain task is running;
//! the drain drives the real session runner, broadcasting events live. GET
//! /events replays persisted events after a cursor, then forwards the live
//! broadcast — so any process (or browser tab) sees a consistent stream.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Result;
use opencoder_core::Config;
use opencoder_llm::{ChatClient, ChatStream};
use opencoder_session::compaction;
use opencoder_session::handoff;
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
    /// pre-subscribe gap 桥接：近期已广播事件的环形快照。
    ///
    /// SSE 客户端通常在 POST /prompt 之后才建立连接，而 drain 侧的事件落库走
    /// 异步 flusher（`event_sink` 对 delta 攒批），因此存在一个窗口：事件已
    /// `broadcast` 但客户端尚未 subscribe、且回放查询 `events_after` 执行时还
    /// 未落库 —— 该事件对这条连接永久丢失（实测 reasoning_delta 丢失导致直播
    /// 态布局与 done 后快照重建不一致）。此 ring 让订阅者在 subscribe 原子地
    /// 拿到「已广播」快照，由 `get_events` 补发其中未被回放覆盖的条目。
    ///
    /// 容量与 `event_sink::CAPACITY`(4096) 对齐：flusher 批次上限 512，环形
    /// 缓冲只需覆盖 flusher 攒批滞后 + 订阅延迟，4096 足以兜住整个在途 turn。
    pub recent: std::sync::Mutex<VecDeque<SseEvt>>,
    pub cancel: Mutex<CancellationToken>,
    pub overrides: Mutex<RuntimeOverrides>,
    pub draining: AtomicBool,
    /// Serializes idle-only mutations with every false -> true drain start.
    /// The atomic remains the cheap status signal; this mutex closes the
    /// check/write/start TOCTOU window.
    pub lifecycle: Arc<Mutex<()>>,
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
    /// Question/answer rendezvous for the `question` tool. Created ONCE here
    /// (stable across drains) — every drain rebinds the resumed session's hub
    /// to this instance, so answers/skips posted between drains are never lost
    /// to a throwaway hub, and the question endpoints always talk to the hub
    /// the (possibly future) drain will use.
    pub question_hub: Arc<opencoder_session::QuestionHub>,
}

const BROADCAST_CAPACITY: usize = 256;

/// 近期广播环形缓冲容量：与 `event_sink::CAPACITY`(4096) 对齐。flusher 的
/// delta 批次上限是 512（条数）/8KB，环形缓冲只需覆盖「flusher 攒批滞后 +
/// 订阅延迟」即可保证不丢，4096 与 flusher channel 同量级兜底。
const RING_CAP: usize = 4096;

impl SessionHandle {
    /// 广播一条 SSE 事件：先入 ring、再发直播流。
    ///
    /// 锁序是关键：append(ring) 与 send(tx) 在同一把 `recent` 锁内完成，而
    /// `subscribe_recent` 的「subscribe(tx) + ring 快照」也持同一把锁，两者
    /// 互斥。由此保证：subscribe 之前广播的事件必然已写进快照（不会丢），
    /// subscribe 之后广播的事件必然只走直播流（不会因快照双发）。
    /// `broadcast::Sender::send` 本身非阻塞，锁内调用安全。
    pub fn broadcast_evt(&self, sse: SseEvt) {
        let mut ring = self.recent.lock().expect("recent ring poisoned");
        ring.push_back(sse.clone());
        while ring.len() > RING_CAP {
            ring.pop_front();
        }
        let _ = self.tx.send(sse);
    }

    /// 订阅直播流并原子地取得 ring 快照（见 `broadcast_evt` 的锁序说明）。
    /// 快照返回后调用方自行用回放窗口做指纹/seq 去重，只补发未落库条目。
    pub fn subscribe_recent(&self) -> (broadcast::Receiver<SseEvt>, Vec<SseEvt>) {
        let ring = self.recent.lock().expect("recent ring poisoned");
        (self.tx.subscribe(), ring.iter().cloned().collect())
    }
}

impl SessionHandle {
    pub fn new() -> Arc<Self> {
        let (tx, _rx) = broadcast::channel::<SseEvt>(BROADCAST_CAPACITY);
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<DrainCmd>();
        Arc::new(SessionHandle {
            tx,
            recent: std::sync::Mutex::new(VecDeque::new()),
            cancel: Mutex::new(CancellationToken::new()),
            overrides: Mutex::new(RuntimeOverrides::default()),
            draining: AtomicBool::new(false),
            lifecycle: Arc::new(Mutex::new(())),
            subscribers: AtomicUsize::new(0),
            cmd_tx,
            cmd_rx: std::sync::Mutex::new(Some(cmd_rx)),
            child_turn_cancels: Arc::new(std::sync::Mutex::new(HashMap::new())),
            child_steer_gates: Arc::new(std::sync::Mutex::new(HashMap::new())),
            child_cancels: Arc::new(std::sync::Mutex::new(HashMap::new())),
            turn_cancel: Arc::new(std::sync::Mutex::new(CancellationToken::new())),
            question_hub: opencoder_session::QuestionHub::new(),
        })
    }
}

/// Failure from a prompt admission that may be idle-only.
#[derive(Debug)]
pub enum AdmissionError {
    BusyModeSwitch,
    Other(anyhow::Error),
}

impl std::fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BusyModeSwitch => write!(f, "mode switch refused while drain running"),
            Self::Other(error) => write!(f, "{error:#}"),
        }
    }
}

impl std::error::Error for AdmissionError {}

impl From<anyhow::Error> for AdmissionError {
    fn from(error: anyhow::Error) -> Self {
        Self::Other(error)
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
    pub(crate) fn new(stream: S, on_drop: impl FnOnce() + Send + Sync + 'static) -> Self {
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

pub(crate) use crate::handle_lifecycle::{
    broadcast_persist_event, ensure_run_error_frame, release_events_subscriber,
};

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
    admit_and_drain_guarded(
        handles, store, session_id, prompt, images, delivery, client, workdir, config, None, false,
    )
    .await
    .map_err(anyhow::Error::new)
}

/// Atomically admit a prompt and optional skill. An idle-only request has no
/// side effects when a drain is active.
///
/// `agent_override` marks a prompt that rewrites the session's agent config
/// (the `agent` field): it is idle-only — refused with
/// [`AdmissionError::BusyModeSwitch`] while a drain is active so a mid-run
/// override never lands. Textual mode commands are NOT overrides: they are
/// admitted and applied by the runner at the next idle/turn boundary.
#[allow(clippy::too_many_arguments)]
pub async fn admit_and_drain_guarded(
    handles: HandleMap,
    store: Arc<dyn Store>,
    session_id: &str,
    prompt: String,
    images: Vec<String>,
    delivery: Delivery,
    client: Arc<dyn ChatStream>,
    workdir: std::path::PathBuf,
    config: Config,
    skill: Option<String>,
    agent_override: bool,
) -> std::result::Result<i64, AdmissionError> {
    let (handle, lifecycle) =
        crate::handle_lifecycle::lock_session_lifecycle(&handles, session_id).await;
    if agent_override && handle.draining.load(Ordering::SeqCst) {
        return Err(AdmissionError::BusyModeSwitch);
    }
    if let Some(skill) = skill {
        store
            .update_session(
                session_id,
                &SessionPatch {
                    skill: Some(skill),
                    updated_at: Some(opencoder_core::message::now_ms()),
                    ..Default::default()
                },
            )
            .await
            // Keep the "persist skill" wording the 500 contract carries
            // (store_error_surfacing.rs) after the admission refactor.
            .map_err(|e| anyhow::anyhow!("persist skill: {e:#}"))?;
    }
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
    let started_new_drain = start_drain_locked(
        handles.clone(),
        store.clone(),
        session_id,
        client.clone(),
        workdir.clone(),
        config.clone(),
        &handle,
    )
    .await;
    drop(lifecycle);
    if !started_new_drain {
        // Steers interrupt the current turn; queued inputs wait for idle.
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
            // Poll for at most ten minutes, then defensively restart if an
            // admitted input was stranded by a failed drain.
            for _ in 0..12_000 {
                if !handle_w.draining.load(Ordering::SeqCst) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            if handle_w.draining.load(Ordering::SeqCst) {
                return;
            }
            let (restart_handle, _lifecycle) =
                crate::handle_lifecycle::lock_session_lifecycle(&handles_w, &sid_w).await;
            if restart_handle.draining.load(Ordering::SeqCst) {
                return;
            }
            // Both delivery channels must be empty before staying idle.
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
            // A hard interrupt wins: keep pending rows for the next explicit
            // prompt instead of resurrecting the cancelled run.
            if restart_handle.cancel.lock().await.is_cancelled() {
                return;
            }
            start_drain_locked(
                handles_w,
                store_w,
                &sid_w,
                client_w,
                wd_w,
                cfg_w,
                &restart_handle,
            )
            .await;
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
    let (handle, _lifecycle) =
        crate::handle_lifecycle::lock_session_lifecycle(&handles, session_id).await;
    start_drain_locked(handles, store, session_id, client, workdir, config, &handle).await;
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn start_drain_locked(
    handles: HandleMap,
    store: Arc<dyn Store>,
    session_id: &str,
    client: Arc<dyn ChatStream>,
    workdir: std::path::PathBuf,
    config: Config,
    handle: &Arc<SessionHandle>,
) -> bool {
    if !handle.draining.swap(true, Ordering::SeqCst) {
        let token = CancellationToken::new();
        *handle.cancel.lock().await = token.clone();
        if let Ok(mut g) = handle.turn_cancel.lock() {
            *g = CancellationToken::new();
        }
        let sid = session_id.to_string();
        let handle_clone = handle.clone();
        tokio::spawn(async move {
            drain_to_completion(handles, store, &sid, client, workdir, config, handle_clone).await;
        });
        true
    } else {
        false
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
    handle: &SessionHandle,
    sink: &opencoder_session::EventSink,
    sid: &str,
    workdir: &std::path::Path,
) {
    // Mirror the main `run` callback: broadcast to live SSE subscribers AND
    // persist the event to the `session_events` table via the sink. Without
    // the `sink.push`, drain-command events (Compaction, Done,
    // TranscriptReset, …) would never reach disk, so an SSE
    // reconnect replay (`?after=<seq>`) would silently miss them.
    // 广播走 `broadcast_evt`（ring + 直播），与主回调同一条 pre-subscribe
    // gap 桥接路径。
    let mut broadcast = |ev: SessionEvent| {
        let (sse, _) = sse_from_session_event(sid, &ev);
        handle.broadcast_evt(sse);
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
        // Execution handoff (web parity of the autopilot ACT phase): collapse
        // the transcript to the newest assistant brief as a single synthetic
        // directive, persist the boundary, and switch back to `act`.
        DrainCmd::Handoff { extra } => {
            if handoff::reset_to_directive(session, &extra).is_some() {
                if let Some(store) = &session.store {
                    let _ = store
                        .update_session(
                            &session.id,
                            &SessionPatch {
                                agent: Some("act".into()),
                                handoff_seq: session.handoff_seq,
                                handoff_plan: session.handoff_plan.clone(),
                                clear_summary: true,
                                clear_skill: true,
                                updated_at: Some(opencoder_core::message::now_ms()),
                                ..Default::default()
                            },
                        )
                        .await;
                }
                broadcast(SessionEvent::TranscriptReset(session.messages.clone()));
                broadcast(SessionEvent::Done);
            } else {
                broadcast(SessionEvent::Error(
                    "nothing to hand off: no assistant reply yet".into(),
                ));
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
        // The three web-parity commands mutate bookkeeping state only (no
        // transcript-visible effect), so unlike Compact/Handoff they neither
        // broadcast nor persist events. Bodies live in `handle_questions.rs`
        // to keep this file within its size budget.
        DrainCmd::SetApMode(mode) => {
            crate::handle_questions::apply_set_ap_mode(session, mode).await;
        }
        DrainCmd::SetAnnotation(text) => {
            crate::handle_questions::apply_set_annotation(session, text).await;
        }
    }
}

/// Drain all pending commands from the receiver and apply them in order.
async fn process_drain_cmds(
    session: &mut opencoder_session::SessionState,
    rx_guard: &mut CmdRxGuard,
    handle: &SessionHandle,
    sink: &opencoder_session::EventSink,
    sid: &str,
    workdir: &std::path::Path,
) {
    if let Some(rx) = rx_guard.rx.as_mut() {
        while let Ok(cmd) = rx.try_recv() {
            apply_drain_cmd(session, cmd, handle, sink, sid, workdir).await;
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
            // TUI worker contract (worker.rs): a drain that cannot even start
            // still owes its SSE subscribers a terminal frame. Without this
            // broadcast the stream hangs open with no Error and no Done while
            // `draining` resets below — a silently dead UI.
            broadcast_persist_event(
                &store,
                &handle,
                session_id,
                SessionEvent::Error(format!("drain: resume failed: {e:#}")),
            )
            .await;
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
            let still_current = map.get(session_id).is_some_and(|h| Arc::ptr_eq(h, &handle));
            let live =
                handle.subscribers.load(Ordering::SeqCst) > 0 || handle.tx.receiver_count() > 0;
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
    // Rebind the question hub to the handle's stable instance and mark a web
    // listener as attached: `resume_session` builds a fresh (unattached) hub,
    // which would make every `question` tool call fall back to
    // NO_LISTENER_REPLY. The runner's registry (runner/registry.rs) builds the
    // `question` tool from `session.question_hub` inside `run()`, which is
    // invoked AFTER this swap — so the tool gets exactly this handle's hub,
    // letting the /questions endpoints answer it mid-turn.
    session.question_hub = handle.question_hub.clone();
    handle.question_hub.attach();

    // 广播句柄克隆进回调：`broadcast_evt` 同时写 ring（pre-subscribe gap
    // 桥接）与直播通道，取代裸 `tx.send`。
    let bcast = Arc::clone(&handle);
    let sid = session_id.to_string();
    let (sink, flusher) =
        opencoder_session::spawn_event_flusher(Some(store.clone()), session_id.to_string());
    // Zero-resubmit: a failed drain NEVER auto-resubmits pending inputs and
    // fires no additional LLM requests. Pending steer/queue rows stay in the
    // store (the admit POST's durable promise) and are consumed by the NEXT
    // successful drain instead of being silently retried inside this failing
    // one. Deliberate semantic change of the former bounded drain-restart
    // loop; the drops below run exactly once, after this single attempt.
    let mut run_emitted_error = false;
    let result = run(&mut session, String::new(), |ev| {
        if matches!(ev, SessionEvent::Error(_)) {
            run_emitted_error = true;
        }
        let (sse, _kind) = sse_from_session_event(&sid, &ev);
        bcast.broadcast_evt(sse);
        let _ = sink.push(&ev);
    })
    .await;

    // Terminal-frame guarantee BEFORE drain commands run: a failed run must
    // surface an `error` frame even when the runner emitted none (see
    // ensure_run_error_frame). Emitted first so no later Done can mask it.
    ensure_run_error_frame(&store, &handle, &sid, &result, run_emitted_error).await;

    // Apply endpoint-forwarded drain commands (autopilot/annotation/...)
    // once the run settles.
    process_drain_cmds(&mut session, &mut rx_guard, &handle, &sink, &sid, &workdir).await;

    // Best-effort title generation after the FIRST successful completion of a
    // drain (mirrors `crates/cli/src/run.rs`): runs while the event sink is
    // still alive but after the run loop breaks, bounded at 30 s so a hanging
    // small-model endpoint can never wedge teardown. `result.is_ok()` gates
    // it to successful runs, and the title check inside makes it once-only
    // across a session's many drains. Failures only log.
    crate::handle_questions::maybe_generate_title(&store, &session, result.is_ok()).await;

    drop(sink);
    if let Err(e) = flusher.await {
        warn!(session_id, error = %e, "final event flush failed");
    }
    // flusher 已排空：此刻 ring 中所有条目必然已落库（或已按丢批策略放弃），
    // 回放可全覆盖，清空 ring。否则一次空闲期重连（after=最新 seq、回放为
    // 空）会把上一 turn 的广播尾巴当「新事件」整体重发。ring 的职责只是
    // 覆盖 flusher 攒批滞后 + 订阅延迟，drain 收束后即失去意义。
    if let Ok(mut ring) = handle.recent.lock() {
        ring.clear();
    }
    drop(rx_guard);
    if let Err(e) = result {
        warn!(session_id, error = %e, "drain ended with error");
    }
    // Keep `draining` true through every teardown step. Clearing it before
    // the event flusher finished and `cmd_rx` was restored opened a tail race:
    // an idle-only mutation (or a fresh drain) could start against a task that
    // had not actually relinquished all of its per-session resources yet.
    drop(guard);
}

#[cfg(test)]
#[path = "handle_tests.rs"]
mod tests;
