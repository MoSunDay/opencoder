//! TUI-side sidecar actor: a resident task that owns the `/sidecar` Q/A
//! panel for ONE session.
//!
//! Contract:
//! - Created once per session (`app.rs` before the worker spawn, and again by
//!   `app_task::switch_session`); a `/task` switch drops the old sender, so
//!   the old actor drains and exits — history never crosses sessions.
//! - The main task is never blocked: questions arrive on a bounded channel
//!   (`try_send` from the UI loop), bypassing steer/queue/prompt entirely.
//! - EPHEMERAL by design: entering the panel ([`enter_panel`]) and leaving it
//!   ([`exit_panel`]) both send [`SidecarCmd::Reset`]. The actor aborts any
//!   in-flight turn, drops its [`SidecarConv`] AND discards the queued
//!   follow-up backlog, so every fresh question rebuilds the conversation
//!   from a FRESH store snapshot (no stale context) and the main transcript
//!   keeps zero sidecar trace.
//! - Zero sidecar persistence: `SidecarStart`/`Child`/`Turn` frames are
//!   display-only. The child's bare `LlmUsage` events ARE persisted to the
//!   main session after each turn — full or aborted-partial (cost accounting
//!   for web replay), matching `worker::persist_event`'s shape.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use opencoder_session::{new_conv_from, run_sidecar_turn, SessionEvent, SessionState, SidecarConv};
use opencoder_store::Store;
use tokio::sync::mpsc;

use ratatui::text::Line;

use crate::chat::{ChatBlock, ChatView, SidecarPanel};
use crate::worker::{persist_event, UiEvent};

/// Channel depth for pending sidecar commands. Small by design: the actor
/// answers serially, and a deep backlog would surprise the user.
const ASK_CHANNEL_CAPACITY: usize = 8;

/// UI -> actor command. `Ask` runs a turn on the current conversation
/// (building it from a fresh snapshot first, if none exists); `Reset`
/// destroys the conversation — aborting an in-flight turn — so the next
/// `Ask` starts over.
pub(crate) enum SidecarCmd {
    Ask(String),
    Reset,
}

/// Spawn the resident sidecar actor for `session`. Returns the command
/// sender. The actor exits once every sender clone is dropped (e.g. after a
/// `/task` switch).
pub(crate) fn spawn_actor(
    session: &SessionState,
    evt_tx: mpsc::Sender<UiEvent>,
    store: Option<Arc<dyn Store>>,
) -> mpsc::Sender<SidecarCmd> {
    // Capture everything the actor needs before the session moves into the
    // worker task (`SessionState` is not `Clone` by design).
    let config = session.config.clone();
    let client = session.client.clone();
    let working_dir = session.working_dir.clone();
    let session_id = session.id.clone();
    let (ask_tx, mut ask_rx) = mpsc::channel::<SidecarCmd>(ASK_CHANNEL_CAPACITY);
    tokio::spawn(async move {
        // One conversation per actor; follow-ups continue it. A `Reset`
        // drops it, so the next `Ask` rebuilds from a fresh snapshot.
        let mut conv: Option<SidecarConv> = None;
        // Asks received while a turn is racing get stashed here and run on
        // the same conversation afterwards.
        let mut backlog: VecDeque<String> = VecDeque::new();
        'actor: loop {
            let question = if let Some(q) = backlog.pop_front() {
                q
            } else {
                match ask_rx.recv().await {
                    Some(SidecarCmd::Ask(q)) => {
                        let q = q.trim().to_string();
                        if q.is_empty() {
                            continue; // focus-only ask: nothing to run
                        }
                        q
                    }
                    Some(SidecarCmd::Reset) => {
                        // Destroy now: an idle reset drops the conversation
                        // AND any queued follow-ups; a running one is handled
                        // by the racing loop below.
                        conv = None;
                        backlog.clear();
                        continue;
                    }
                    None => break 'actor, // every sender dropped: exit
                }
            };
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
            let Some(active) = conv.take() else {
                continue;
            };
            // Collect the bare usage records this turn produces so they can
            // be persisted after the turn (the FnMut callback cannot await).
            // The Arc survives an abort: a destroyed in-flight turn still
            // owes the main session its partial cost.
            let usage: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
            let usage_for_cb = Arc::clone(&usage);
            let tx_for_cb = evt_tx.clone();
            let evt_tx_for_err = evt_tx.clone();
            let turn_id = active.id.clone();
            let id_for_err = turn_id.clone();
            let q = question.clone();
            let mut handle = tokio::spawn(async move {
                let mut active = active;
                let mut on_event = move |ev: SessionEvent| {
                    if let SessionEvent::LlmUsage { .. } = &ev {
                        if let Ok(mut g) = usage_for_cb.lock() {
                            g.push(ev.clone());
                        }
                    }
                    // Display path: best-effort; the bounded UI channel may
                    // shed deltas under pressure (usage records persist
                    // regardless).
                    let _ = tx_for_cb.try_send(UiEvent::Session(ev));
                };
                if run_sidecar_turn(&mut active, &q, &mut on_event)
                    .await
                    .is_err()
                {
                    // The turn's own sink normally reports the outcome; a
                    // hard actor-level failure still owes the UI a terminal
                    // frame.
                    let _ = evt_tx_for_err
                        .send(UiEvent::Session(SessionEvent::SidecarTurn {
                            id: id_for_err,
                            ok: false,
                            answer: "sidecar turn failed".to_string(),
                            elapsed_ms: 0,
                            total_tokens: 0,
                            rounds: 0,
                        }))
                        .await;
                }
                active
            });
            // Race the turn against incoming commands: a `Reset` (or actor
            // shutdown) aborts the in-flight turn — its partial usage is
            // persisted below, its content frames simply stop. A queued
            // follow-up is stashed for the outer loop instead of waiting for
            // the channel.
            let mut finished: Option<SidecarConv> = None;
            let mut shutdown = false;
            loop {
                #[allow(clippy::large_enum_variant)] // local, short-lived
                enum TurnEnd {
                    Done(Result<SidecarConv, tokio::task::JoinError>),
                    Cmd(Option<SidecarCmd>),
                }
                let end = tokio::select! {
                    res = &mut handle => TurnEnd::Done(res),
                    next = ask_rx.recv() => TurnEnd::Cmd(next),
                };
                match end {
                    TurnEnd::Done(res) => {
                        finished = res.ok();
                        break;
                    }
                    TurnEnd::Cmd(None) => {
                        handle.abort();
                        let _ = handle.await; // join: the usage collector is now final
                        shutdown = true;
                        break;
                    }
                    TurnEnd::Cmd(Some(SidecarCmd::Reset)) => {
                        handle.abort();
                        let _ = handle.await; // join: the usage collector is now final
                                              // Destroy = destroy EVERYTHING: a queued follow-up
                                              // must not outlive the destroyed panel, or it would
                                              // rebuild the conversation and keep burning tokens
                                              // after the user left.
                        backlog.clear();
                        break;
                    }
                    TurnEnd::Cmd(Some(SidecarCmd::Ask(q))) => {
                        let q = q.trim().to_string();
                        if !q.is_empty() {
                            backlog.push_back(q);
                        }
                    }
                }
            }
            // Cost accounting: persist the turn's bare LlmUsage records onto
            // the MAIN session — never the sidecar's own frames. An aborted
            // turn keeps whatever usage it already produced.
            let collected: Vec<SessionEvent> = usage
                .lock()
                .map(|mut g| g.drain(..).collect())
                .unwrap_or_default();
            for ev in collected {
                persist_event(&store, &session_id, &ev).await;
            }
            match finished {
                Some(c) => conv = Some(c), // follow-ups continue this conversation
                None => conv = None,       // destroyed / aborted: rebuild next ask
            }
            if shutdown {
                break 'actor;
            }
        }
    });
    ask_tx
}

// ── Panel entry / exit (the destroy-on-entry, destroy-on-exit contract) ────

/// Best-effort [`SidecarCmd::Reset`] delivery. The only possible failure is
/// `Full` (the actor owns its receiver, so `Closed` cannot happen while this
/// sender lives) and losing a `Reset` there is benign: the caller purges the
/// panel UI either way, and the stale conversation is destroyed by the next
/// entry/exit `Reset`. No retry: the UI loop must never block on the actor.
fn send_reset(ask: &mpsc::Sender<SidecarCmd>) {
    let _ = ask.try_send(SidecarCmd::Reset);
}

/// Enter the sidecar panel on a clean slate. Entry ALWAYS destroys the
/// previous sidecar conversation: the actor is told to drop its
/// [`SidecarConv`] (aborting an in-flight turn; partial usage still lands on
/// the main session), the panel field is purged and an empty panel takes
/// focus. The next `Ask` rebuilds the conversation from a fresh store
/// snapshot — which is why re-entering never shows stale context.
pub(crate) fn enter_panel(chat: &mut ChatView, ask: &mpsc::Sender<SidecarCmd>) {
    send_reset(ask);
    crate::chat::sidecar::purge(chat);
    // Empty panel anchor: the panel lives in `chat.sidecar`, NOT in
    // `blocks`, so in-flight main-task streaming blocks stay tail-merged
    // while the panel is open. Real conversations always start with the
    // `sidecar-` prefix, so the empty `id` marks it fresh for the first
    // `SidecarStart` frame to adopt in place.
    chat.sidecar = Some(SidecarPanel {
        id: String::new(),
        question: String::new(),
        view: Box::new(ChatView::default()),
        done: false,
        ok: false,
        answer: None,
        total_tokens: 0,
        rounds: 0,
        started_at_ms: opencoder_core::message::now_ms(),
        elapsed_ms: 0,
    });
    chat.sidecar_focus = true;
}

/// Exit the sidecar panel (ESC / Ctrl+L): destroy, don't hide. The actor
/// drops its conversation (aborting an in-flight turn) and the panel field
/// is cleared, so the main view carries zero sidecar trace afterwards.
pub(crate) fn exit_panel(chat: &mut ChatView, ask: &mpsc::Sender<SidecarCmd>) {
    send_reset(ask);
    crate::chat::sidecar::purge(chat);
}

/// Ask channel full or actor gone: retry shortly.
pub(crate) const SIDECAR_BUSY_FLASH: &str = "⏳ sidecar busy — retry in a moment";

/// Echo a submitted question into the panel's nested view so the user sees
/// it the moment they press Enter. The actor must first load a store
/// snapshot and build its [`SidecarConv`] before the first `SidecarStart`
/// frame adopts the placeholder — without this echo the question is
/// invisible for that whole beat (the "submitted and stuck" feel). Mirrors
/// the main transcript's `push_user`: markdown-rendered `ChatBlock::User` +
/// blank marker. `SidecarStart` adoption keeps the nested view, so the echo
/// survives into the titled panel exactly once (frames never re-echo); no
/// open panel (late exit / purge) is a no-op.
pub(crate) fn echo_question(chat: &mut ChatView, question: &str) {
    if let Some(panel) = chat.sidecar.as_mut() {
        panel.view.blocks.push(ChatBlock::User {
            rendered: crate::markdown::render(question),
        });
        panel.view.push_marker(Line::from(""));
        // The echo is this panel-turn's anchor: re-anchor the ladder floor
        // BELOW it (mirrors the main transcript's queue-consumed echo) so the
        // turn's `N Steps` group renders after the prompt — never above it.
        panel.view.reanchor_turn_after_user_echo();
    }
}
