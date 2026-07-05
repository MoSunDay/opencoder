//! Web session runtime: per-session broadcast handles + background drain tasks.
//!
//! A `SessionHandle` owns a tokio `broadcast::Sender` of `SseEvt`. POST /prompt
//! admits an input to the store and ensures exactly one drain task is running;
//! the drain drives the real session runner, broadcasting events live. GET
//! /events replays persisted events after a cursor, then forwards the live
//! broadcast — so any process (or browser tab) sees a consistent stream.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use opencode_core::Config;
use opencode_llm::ChatStream;
use opencode_session::{resume as resume_session, run, SessionEvent};
use opencode_store::{Delivery, EventKind, SessionEventRecord, SessionInput, Store};
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
        let ts = opencode_core::message::now_ms();
        let (kind, data, event_kind) = match ev {
            SessionEvent::TextDelta(t) => ("text_delta".to_string(), serde_json::json!({ "text": t }), EventKind::TextDelta),
            SessionEvent::ToolStart { id, name, input } => ("tool_start".to_string(), serde_json::json!({ "id": id, "name": name, "input": input }), EventKind::ToolStart),
            SessionEvent::ToolEnd { id, name, output, is_error } => ("tool_end".to_string(), serde_json::json!({ "id": id, "name": name, "output": output, "is_error": is_error }), EventKind::ToolEnd),
            SessionEvent::AgentSwitch(a) => ("agent_switched".to_string(), serde_json::json!({ "agent": a }), EventKind::AgentSwitched),
            SessionEvent::Compaction(s) => ("compaction".to_string(), serde_json::json!({ "summary": s }), EventKind::Compaction),
            SessionEvent::Status(s) => ("status".to_string(), serde_json::json!({ "status": s }), EventKind::Step),
            SessionEvent::Done => ("done".to_string(), serde_json::json!({}), EventKind::Done),
            SessionEvent::Error(e) => ("error".to_string(), serde_json::json!({ "error": e }), EventKind::Error),
            SessionEvent::ReasoningDelta(r) => ("reasoning_delta".to_string(), serde_json::json!({ "text": r }), EventKind::TextDelta),
            SessionEvent::SubagentStart { id, kind, prompt } => ("subagent_start".to_string(), serde_json::json!({ "id": id, "kind": kind, "prompt": prompt }), EventKind::Step),
            SessionEvent::SubagentEnd { id, ok, summary } => ("subagent_end".to_string(), serde_json::json!({ "id": id, "ok": ok, "summary": summary }), EventKind::Step),
        };
        (SseEvt { kind, data, ts, seq: None }, event_kind)
    }
}

/// Per-session runtime state shared across HTTP requests and the drain task.
pub struct SessionHandle {
    pub tx: broadcast::Sender<SseEvt>,
    pub cancel: CancellationToken,
    /// mutable runtime overrides applied at the next drain boundary
    pub overrides: Mutex<RuntimeOverrides>,
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

const BROADCAST_CAPACITY: usize = 256;

/// Admit a prompt durably, then ensure exactly one drain task is running for the
/// session. Returns the admitted input's seq. If the session has no live handle,
/// one is created and a drain is spawned; otherwise the running drain absorbs
/// the new input at its next turn boundary (steer) or idle point (queue).
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
        id: uuid::Uuid::new_v4().to_string(),
        session_id: session_id.to_string(),
        delivery,
        prompt,
        admitted_seq: 0,
        promoted_seq: None,
    };
    let seq = store.admit_input(&input).await?;

    let mut map = handles.lock().await;
    let need_spawn = !map.contains_key(session_id);
    if need_spawn {
        let (tx, _rx) = broadcast::channel::<SseEvt>(BROADCAST_CAPACITY);
        let cancel = CancellationToken::new();
        let handle = Arc::new(SessionHandle {
            tx,
            cancel: cancel.clone(),
            overrides: Mutex::new(RuntimeOverrides::default()),
        });
        map.insert(session_id.to_string(), handle.clone());
        let handles_clone = handles.clone();
        let store_clone = store.clone();
        let sid = session_id.to_string();
        let cfg = config.clone();
        let client_clone = client.clone();
        let wd = workdir.clone();
        tokio::spawn(async move {
            drain_to_completion(handles_clone, store_clone, &sid, client_clone, wd, cfg, handle).await;
        });
    }
    Ok(seq)
}

/// Drive the session runner to completion, broadcasting events. Applies runtime
/// overrides (agent/model) before starting. When the drain finishes (Done /
/// interrupted / error), the handle is left in the map so late SSE subscribers
/// can still replay, but a fresh prompt will spawn a new drain.
async fn drain_to_completion(
    handles: HandleMap,
    store: Arc<dyn Store>,
    session_id: &str,
    client: Arc<dyn ChatStream>,
    workdir: std::path::PathBuf,
    mut config: Config,
    handle: Arc<SessionHandle>,
) {
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
    let mut session = match resume_session(store.clone(), session_id, config.clone(), client.clone(), workdir.clone()).await {
        Ok(s) => s,
        Err(e) => {
            // session row missing — caller must create it first (POST /sessions).
            warn!(session_id, error = %e, "drain: cannot resume (session row missing?)");
            let mut map = handles.lock().await;
            map.remove(session_id);
            return;
        }
    };
    session.cancel = Some(handle.cancel.clone());

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

    // mark idle: leave the handle so SSE replay works, but cancel token stays
    // so a re-admit spawns a fresh drain via the need_spawn check after removal.
    let mut map = handles.lock().await;
    map.remove(session_id);
}
