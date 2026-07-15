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
use opencoder_llm::ChatStream;
use opencoder_session::{resume as resume_session, run, SessionEvent};
use opencoder_store::{Delivery, EventKind, SessionEventRecord, SessionInput, Store};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::warn;

/// A single SSE event delivered to subscribers (and persisted for replay).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SseEvt {
    pub kind: String,
    pub data: serde_json::Value,
    pub ts: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,
}

impl SseEvt {
    pub fn from_session_event(_session_id: &str, ev: &SessionEvent) -> (Self, EventKind) {
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
}

/// Per-session runtime state shared across HTTP requests, SSE subscribers, and
/// the background drain task. A handle is get-or-created by either `/events`
/// (to share the broadcast channel with future live events) or `/prompt` (to
/// drive a drain). Whether a drain is *actually running* is tracked by
/// `draining`, NOT by map presence — otherwise an early SSE subscriber's handle
/// would block the drain from spawning (`/prompt` would admit then never run).
pub struct SessionHandle {
    pub tx: broadcast::Sender<SseEvt>,
    /// Per-drain cancel token, refreshed on each spawn so a prior interrupt's
    /// permanent cancellation can't poison a subsequent drain.
    pub cancel: Mutex<CancellationToken>,
    /// mutable runtime overrides applied at the next drain boundary
    pub overrides: Mutex<RuntimeOverrides>,
    /// CAS guard: the first caller to flip this `true` owns the drain spawn.
    /// Cleared by `DrainGuard` when the drain ends (normal/early/panic).
    pub draining: AtomicBool,
}

const BROADCAST_CAPACITY: usize = 256;

impl SessionHandle {
    /// Fresh handle with a non-cancelled token and `draining = false`.
    pub fn new() -> Arc<Self> {
        let (tx, _rx) = broadcast::channel::<SseEvt>(BROADCAST_CAPACITY);
        Arc::new(SessionHandle {
            tx,
            cancel: Mutex::new(CancellationToken::new()),
            overrides: Mutex::new(RuntimeOverrides::default()),
            draining: AtomicBool::new(false),
        })
    }
}

#[derive(Default)]
pub struct RuntimeOverrides {
    pub agent: Option<String>,
    pub model: Option<String>,
}

/// The registry of live session handles, keyed by session id.
pub type HandleMap = Arc<Mutex<HashMap<String, Arc<SessionHandle>>>>;

pub fn new_handle_map() -> HandleMap {
    Arc::new(Mutex::new(HashMap::new()))
}

/// RAII guard that clears the handle's `draining` flag on drop. Covers normal
/// completion, early returns, and task panic (tokio unwinds spawned futures so
/// `Drop` still runs), ensuring a crashed/idle drain can be re-spawned.
struct DrainGuard {
    handle: Arc<SessionHandle>,
}

impl Drop for DrainGuard {
    fn drop(&mut self) {
        self.handle.draining.store(false, Ordering::SeqCst);
    }
}

/// Admit a prompt durably, then ensure exactly one drain task is running for the
/// session. Returns the admitted input's seq. A handle is get-or-created to
/// share the broadcast channel (so early SSE subscribers receive live events);
/// a `draining` CAS picks the single drain owner, independent of handle
/// presence. A running drain absorbs the new input at its next turn boundary
/// (steer) or idle point (queue).
#[allow(clippy::too_many_arguments)]
pub async fn admit_and_drain(
    handles: HandleMap,
    store: Arc<dyn Store>,
    session_id: &str,
    prompt: String,
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
        admitted_seq: 0,
        promoted_seq: None,
    };
    let seq = store.admit_input(&input).await?;

    // get-or-create the channel handle so an early /events subscriber (which
    // may have created it) keeps receiving once the drain broadcasts here.
    let handle = {
        let mut map = handles.lock().await;
        map.entry(session_id.to_string())
            .or_insert_with(SessionHandle::new)
            .clone()
    };

    // CAS: first to flip draining true owns the spawn. Handle presence alone
    // (e.g. a handle created by /events with no drain) must NOT gate spawning,
    // otherwise the prompt is admitted but never processed.
    if !handle.draining.swap(true, Ordering::SeqCst) {
        // refresh the cancel token so a previous interrupt's permanent cancel
        // cannot immediately abort this fresh drain.
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
    Ok(seq)
}

/// Drive the session runner to completion, broadcasting events. Applies runtime
/// overrides (agent/model) before starting. The handle is left in the map when
/// the drain ends (normal or interrupted) so late SSE subscribers can still
/// replay from the store and a fresh prompt spawns a new drain; the `DrainGuard`
/// clears `draining` on any exit path. On a missing session row the handle is
/// removed (nothing is replayable and the caller must POST /sessions first).
async fn drain_to_completion(
    handles: HandleMap,
    store: Arc<dyn Store>,
    session_id: &str,
    client: Arc<dyn ChatStream>,
    workdir: std::path::PathBuf,
    mut config: Config,
    handle: Arc<SessionHandle>,
) {
    // clear `draining` on any exit (including panic) so the session is re-spawnable.
    let _guard = DrainGuard {
        handle: handle.clone(),
    };

    // apply overrides
    {
        let ov = handle.overrides.lock().await;
        if let Some(a) = &ov.agent {
            config.agent.default = a.clone();
        }
        if let Some(m) = &ov.model {
            config.model = m.clone();
        }
    }

    // build the session (resume if history exists)
    let mut session = match resume_session(
        store.clone(),
        session_id,
        config.clone(),
        client.clone(),
        workdir.clone(),
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            // session row missing — caller must create it first (POST /sessions).
            warn!(session_id, error = %e, "drain: cannot resume (session row missing?)");
            let mut map = handles.lock().await;
            map.remove(session_id);
            return;
        }
    };
    session.cancel = Some(handle.cancel.lock().await.clone());

    let tx = handle.tx.clone();
    let store_for_evt = store.clone();
    let sid = session_id.to_string();
    let result = run(&mut session, String::new(), |ev| {
        let (sse, kind) = SseEvt::from_session_event(&sid, &ev);
        // broadcast (ignore lagging subscribers)
        let _ = tx.send(sse.clone());
        // persist for replay
        let rec = SessionEventRecord {
            session_id: sid.clone(),
            kind,
            payload: sse.data.clone(),
            ts: sse.ts,
            seq: None,
            sse_kind: Some(sse.kind.clone()),
        };
        let s2 = store_for_evt.clone();
        let r2 = rec;
        tokio::spawn(async move {
            let _ = s2.append_event(&r2).await;
        });
    })
    .await;

    if let Err(e) = result {
        warn!(session_id, error = %e, "drain ended with error");
    }

    // idle: leave the handle in the map so SSE replay (from the store) and a
    // later re-admit both work; `DrainGuard` has already cleared `draining`.
}
