//! TUI-side sidecar actor (Phase 2): a resident task that owns the
//! `/sidecar <question>` conversation for ONE session.
//!
//! Contract:
//! - Created once per session (`app.rs` before the worker spawn, and again by
//!   `app_task::switch_session`); a `/task` switch drops the old sender, so
//!   the old actor drains and exits — history never crosses sessions.
//! - The main task is never blocked: questions arrive on a bounded channel
//!   (`try_send` from the UI loop), bypassing steer/queue/prompt entirely.
//! - First question snapshots the main transcript from the store, builds a
//!   store-less child loop (`new_conv_from`), and follow-ups reuse the same
//!   [`SidecarConv`] (snapshot + accumulated Q/A history in memory).
//! - Zero sidecar persistence: `SidecarStart`/`Child`/`Turn` frames are
//!   display-only. The child's bare `LlmUsage` events ARE persisted to the
//!   main session after each turn (cost accounting for web replay), matching
//!   `worker::persist_event`'s shape.

use std::sync::{Arc, Mutex};

use opencoder_session::{new_conv_from, run_sidecar_turn, SessionEvent, SessionState, SidecarConv};
use opencoder_store::Store;
use tokio::sync::mpsc;

use crate::worker::{persist_event, UiEvent};

/// Channel depth for pending sidecar questions. Small by design: the actor
/// answers serially, and a deep backlog would surprise the user.
const ASK_CHANNEL_CAPACITY: usize = 8;

/// Payload of the ask channel. Besides a question, the UI loop forwards a
/// `Reset` whenever the main transcript was rebuilt (`TranscriptReset` —
/// compaction / `/act_clear_context`): the live conversation snapshot the
/// pre-reset transcript and its display blocks were wiped from the rebuilt
/// view, so the conversation must be dropped — the next question re-snapshots
/// the fresh transcript and emits a new `SidecarStart` (a new block).
#[derive(Debug, Clone)]
pub(crate) enum SidecarAsk {
    Question(String),
    Reset,
}

/// Spawn the resident sidecar actor for `session`. Returns the ask sender;
/// a [`SidecarAsk::Question`] text must be non-empty (empty is the bare
/// `/sidecar` focus-only form and is handled entirely in the UI layer).
/// The actor exits once every sender clone is dropped (e.g. after a `/task`
/// switch, which also respawns a fresh actor — history never crosses sessions).
pub(crate) fn spawn_actor(
    session: &SessionState,
    evt_tx: mpsc::Sender<UiEvent>,
    store: Option<Arc<dyn Store>>,
) -> mpsc::Sender<SidecarAsk> {
    // Capture everything the actor needs before the session moves into the
    // worker task (`SessionState` is not `Clone` by design).
    let config = session.config.clone();
    let client = session.client.clone();
    let working_dir = session.working_dir.clone();
    let session_id = session.id.clone();
    let (ask_tx, mut ask_rx) = mpsc::channel::<SidecarAsk>(ASK_CHANNEL_CAPACITY);
    tokio::spawn(async move {
        // One conversation per actor; follow-ups continue it until a Reset.
        let mut conv: Option<SidecarConv> = None;
        while let Some(msg) = ask_rx.recv().await {
            let question = match msg {
                SidecarAsk::Reset => {
                    conv = None;
                    continue;
                }
                SidecarAsk::Question(q) => q,
            };
            let question = question.trim().to_string();
            if question.is_empty() {
                continue; // focus-only ask: nothing to run
            }
            // Build the conversation lazily on the first question: snapshot
            // the main transcript so the sidecar can see the parent's context.
            if conv.is_none() {
                let snapshot = match &store {
                    Some(s) => s.load_messages(&session_id).await.unwrap_or_default(),
                    None => Vec::new(),
                };
                match new_conv_from(
                    config.clone(),
                    client.clone(),
                    working_dir.clone(),
                    snapshot,
                )
                .await
                {
                    Ok(c) => {
                        let _ = evt_tx
                            .send(UiEvent::Session(SessionEvent::SidecarStart {
                                id: c.id.clone(),
                                question: question.clone(),
                            }))
                            .await;
                        conv = Some(c);
                    }
                    Err(e) => {
                        // No conversation, no block: surface the failure on
                        // the status line and drop the question.
                        let _ = evt_tx
                            .send(UiEvent::Session(SessionEvent::Status(format!(
                                "sidecar unavailable: {e}"
                            ))))
                            .await;
                        continue;
                    }
                }
            }
            let Some(active) = conv.as_mut() else {
                continue;
            };
            // Collect the bare usage records this turn produces so they can be
            // persisted after the turn (the FnMut callback cannot await).
            let usage: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
            let usage_for_cb = usage.clone();
            // Terminal frames (`SidecarTurn` — the child's own failure is
            // always expressed as `SidecarTurn { ok: false }`) are buffered
            // here and delivered with awaited `send`s after the turn: a
            // lossy `try_send` under UI-channel backpressure would drop the
            // final frame and leave the block stuck "running" forever.
            let terminal: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
            let terminal_for_cb = terminal.clone();
            let tx_for_cb = evt_tx.clone();
            let mut on_event = move |ev: SessionEvent| {
                if matches!(ev, SessionEvent::LlmUsage { .. }) {
                    if let Ok(mut g) = usage_for_cb.lock() {
                        g.push(ev.clone());
                    }
                }
                if matches!(ev, SessionEvent::SidecarTurn { .. }) {
                    if let Ok(mut t) = terminal_for_cb.lock() {
                        t.push(ev);
                    }
                    return;
                }
                // Display path for high-frequency deltas: best-effort; the
                // bounded UI channel may shed them under pressure (usage
                // records persist regardless).
                let _ = tx_for_cb.try_send(UiEvent::Session(ev));
            };
            let turn_failed = run_sidecar_turn(active, &question, &mut on_event)
                .await
                .is_err();
            // Awaited delivery of the terminal frame(s): ordered after the
            // turn's deltas (already queued by the closure above). The buffer
            // is cloned out of the lock first — the guard must never be held
            // across the awaits below (the actor future has to stay Send).
            let terminal_frames: Vec<SessionEvent> =
                terminal.lock().map(|t| t.clone()).unwrap_or_default();
            for ev in terminal_frames {
                let _ = evt_tx.send(UiEvent::Session(ev)).await;
            }
            if turn_failed {
                // The turn's own sink normally reports the outcome; a hard
                // actor-level failure still owes the UI a terminal frame.
                let _ = evt_tx
                    .send(UiEvent::Session(SessionEvent::SidecarTurn {
                        id: active.id.clone(),
                        ok: false,
                        answer: "sidecar turn failed".to_string(),
                        elapsed_ms: 0,
                        total_tokens: 0,
                        rounds: 0,
                    }))
                    .await;
            }
            // Cost accounting: persist the turn's bare LlmUsage records onto
            // the MAIN session — never the sidecar's own frames.
            let collected: Vec<SessionEvent> = usage.lock().map(|g| g.clone()).unwrap_or_default();
            for ev in collected {
                persist_event(&store, &session_id, &ev).await;
            }
        }
    });
    ask_tx
}

// ── Composer flash strings (single source of truth, test-pinned) ────────────

/// Bare `/sidecar` re-focused an existing sidecar box.
pub(crate) const SIDECAR_FOCUSED_FLASH: &str = "\u{2937} sidecar 已聚焦 · 输入即追问";
/// Bare `/sidecar` with no sidecar conversation yet: usage hint.
pub(crate) const SIDECAR_HINT_FLASH: &str = "输入 /sidecar <问题> 提问";
/// Ask channel full or actor gone: retry shortly.
pub(crate) const SIDECAR_BUSY_FLASH: &str = "⏳ sidecar busy — retry in a moment";
