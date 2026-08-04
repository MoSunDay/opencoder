//! Web session runtime: per-session broadcast handles + background drain tasks.
//!
//! A `SessionHandle` owns a tokio `broadcast::Sender` of `SseEvt`. POST /prompt
//! admits an input to the store and ensures exactly one drain task is running;
//! the drain drives the real session runner, broadcasting events live. GET
//! /events replays persisted events after a cursor, then forwards the live
//! broadcast — so any process (or browser tab) sees a consistent stream.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
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
    /// Sender for drain commands (compact, handoff, skill, config reload).
    pub cmd_tx: mpsc::UnboundedSender<DrainCmd>,
    /// Receiver for drain commands. `Option` so the drain task can `take()` it
    /// for exclusive access, then put it back. Panic-safe via `CmdRxGuard`.
    pub cmd_rx: std::sync::Mutex<Option<mpsc::UnboundedReceiver<DrainCmd>>>,
    pub child_turn_cancels:
        Arc<std::sync::Mutex<HashMap<String, opencoder_session::SharedCancel>>>,
    pub child_cancels: Arc<std::sync::Mutex<HashMap<String, CancellationToken>>>,
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
            cmd_tx,
            cmd_rx: std::sync::Mutex::new(Some(cmd_rx)),
            child_turn_cancels: Arc::new(std::sync::Mutex::new(HashMap::new())),
            child_cancels: Arc::new(std::sync::Mutex::new(HashMap::new())),
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

    if !handle.draining.swap(true, Ordering::SeqCst) {
        let token = CancellationToken::new();
        *handle.cancel.lock().await = token.clone();
        let handles_clone = handles.clone();
        let store_clone = store.clone();
        let sid = session_id.to_string();
        let cfg = config.clone();
        let client_clone = client.clone();
        let wd = workdir.clone();
        let handle_clone = handle.clone();
        tokio::spawn(async move {
            drain_to_completion(
                handles_clone, store_clone, &sid, client_clone, wd, cfg, handle_clone,
            )
            .await;
        });
    } else {
        let handles_w = handles.clone();
        let store_w = store.clone();
        let sid_w = session_id.to_string();
        let cfg_w = config.clone();
        let client_w = client.clone();
        let wd_w = workdir.clone();
        let handle_w = handle.clone();
        tokio::spawn(async move {
            for _ in 0..100 {
                if !handle_w.draining.load(Ordering::SeqCst) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            if handle_w.draining.load(Ordering::SeqCst) {
                return;
            }
            let pending = store_w
                .pending_inputs(&sid_w, opencoder_store::Delivery::Queue)
                .await
                .unwrap_or_default();
            if pending.is_empty() {
                return;
            }
            if !handle_w.draining.swap(true, Ordering::SeqCst) {
                let token = CancellationToken::new();
                *handle_w.cancel.lock().await = token.clone();
                drain_to_completion(handles_w, store_w, &sid_w, client_w, wd_w, cfg_w, handle_w)
                    .await;
            }
        });
    }
    let _ = opencoder_session::fire_child_cancels(&handle.child_cancels);
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
        let handles_clone = handles.clone();
        let store_clone = store.clone();
        let sid = session_id.to_string();
        let cfg = config.clone();
        let client_clone = client.clone();
        let wd = workdir.clone();
        let handle_clone = handle.clone();
        tokio::spawn(async move {
            drain_to_completion(
                handles_clone, store_clone, &sid, client_clone, wd, cfg, handle_clone,
            )
            .await;
        });
    }
}

/// Send a drain command to the session's handle.
pub async fn send_cmd(handles: &HandleMap, session_id: &str, cmd: DrainCmd) -> bool {
    let map = handles.lock().await;
    if let Some(h) = map.get(session_id) {
        let _ = h.cmd_tx.send(cmd);
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
    sid: &str,
    workdir: &std::path::Path,
) {
    let mut broadcast = |ev: SessionEvent| {
        let (sse, _) = sse_from_session_event(sid, &ev);
        let _ = tx.send(sse);
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
                broadcast(SessionEvent::Error(
                    "no plan to hand off".into(),
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
                        Ok(c) => session
                            .apply_config_reload(new_cfg, Arc::new(c) as Arc<dyn ChatStream>),
                        Err(_) => session.apply_config_reload_keep_client(new_cfg),
                    },
                    Err(_) => session.apply_config_reload_keep_client(new_cfg),
                }
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
    sid: &str,
    workdir: &std::path::Path,
) {
    if let Some(rx) = rx_guard.rx.as_mut() {
        while let Ok(cmd) = rx.try_recv() {
            apply_drain_cmd(session, cmd, tx, sid, workdir).await;
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
            map.remove(session_id);
            return;
        }
    };
    session.cancel = Some(handle.cancel.lock().await.clone());
    session.child_turn_cancels = handle.child_turn_cancels.clone();
    session.child_cancels = handle.child_cancels.clone();

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
    process_drain_cmds(&mut session, &mut rx_guard, &tx, &sid, &workdir).await;

    drop(sink);
    drop(guard);
    drop(rx_guard);
    if let Err(e) = flusher.await {
        warn!(session_id, error = %e, "final event flush failed");
    }
    if let Err(e) = result {
        warn!(session_id, error = %e, "drain ended with error");
    }
}
