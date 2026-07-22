//! Background worker command processing — shared by the main worker and the
//! `/task`-spawned worker to avoid duplicate match arms.

use std::sync::Arc;

use opencoder_core::{message::now_ms, resolve_agent, Config};
use opencoder_llm::ChatClient;
use opencoder_session::{run as run_session, spawn_event_flusher, SessionEvent, SessionState};
use opencoder_store::{SessionEventRecord, Store};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub enum UiCmd {
    Prompt(String),
    SwitchAgent(String),
    /// Switch agent then immediately start a turn without recording a new user
    /// message. Used for the plan->act manual transition: the system prompt
    /// changes to act and the model reads the plan from conversation history.
    /// The second field carries any text left in the plan-mode input box; it
    /// is appended to the plan during the handoff so it is submitted too.
    SwitchAndStart(String, String),
    /// Manually trigger conversation compaction.
    Compact,
    SetSkill(Option<String>),
    /// Hot-reload config at the next turn boundary. Sent by the `/config` menu.
    ReloadConfig(Box<Config>),
    /// Swap the session's cancellation token for a fresh, uncancelled one.
    /// Sent before every turn-starting command so a prior double-Esc abort
    /// doesn't leave `sess.cancel` permanently cancelled (which would make
    /// `run_loop` break instantly at its top-of-loop `is_cancelled()` check,
    /// silently rejecting all subsequent submissions). The loop reassigns its
    /// own `cancel` handle to a clone of the same token so double-Esc still
    /// targets the live turn.
    ResetCancel(CancellationToken),
    Quit,
}

pub enum UiEvent {
    Session(SessionEvent),
    TurnDone,
}

/// Rebind the main loop's session-scoped handles to a freshly switched session.
///
/// Called after `/task` picks a new/resumed session. All four handles move
/// together: command channel, event stream, session id, AND the cancellation
/// token. The token is load-bearing — double-Esc cancels the loop's `cancel`,
/// so it must point at the active session's worker. Leaving it bound to the
/// first session (regression F1) made `/task`-switched sessions uninterruptable.
#[allow(clippy::too_many_arguments)]
pub fn rebind_session(
    cmd_tx: &mut mpsc::Sender<UiCmd>,
    evt_rx: &mut mpsc::Receiver<UiEvent>,
    session_id: &mut String,
    cancel: &mut CancellationToken,
    new_cmd_tx: mpsc::Sender<UiCmd>,
    new_evt_rx: mpsc::Receiver<UiEvent>,
    new_session_id: String,
    new_cancel: CancellationToken,
) {
    *cmd_tx = new_cmd_tx;
    *evt_rx = new_evt_rx;
    *session_id = new_session_id;
    *cancel = new_cancel;
}

/// `/compact` dispatch policy: only run when idle. Kept as a pure function so
/// the running-guard (and its busy feedback) is unit-testable independent of the
/// async event loop.
#[derive(Debug, PartialEq, Eq)]
pub enum CompactGate {
    Run,
    SkipRunning,
}

pub fn gate_compact(running: bool) -> CompactGate {
    if running {
        CompactGate::SkipRunning
    } else {
        CompactGate::Run
    }
}

/// Gate for the `/task` "Clear all" destructive action. A turn in flight
/// (`running == true`) means a subagent may still be writing to its child
/// session — clearing then would yank that row out from under it (FK
/// violation on the next append). Refuse until idle (all subagents returned).
#[derive(Debug, PartialEq, Eq)]
pub enum ClearAllGate {
    Run,
    SkipRunning,
}

pub fn gate_clear_all(running: bool) -> ClearAllGate {
    if running {
        ClearAllGate::SkipRunning
    } else {
        ClearAllGate::Run
    }
}

/// Minimum free capacity to reserve for lifecycle events. When the channel
/// is near-full, droppable streaming events (TextDelta, ReasoningDelta, and
/// SubagentChild wrapping those) are dropped — their final text is always
/// reconstructed from the store by `TurnDone → finalize_assistant()`, so no
/// information is lost. Non-delta lifecycle events always get a slot.
const DELTA_MIN_CAPACITY: usize = 64;

/// Returns true for events whose information is fully recoverable from the
/// store on the next `TurnDone` (i.e. streaming text deltas). These can be
/// safely dropped when the UI channel is near capacity without data loss.
fn is_droppable_delta(sev: &SessionEvent) -> bool {
    match sev {
        SessionEvent::TextDelta(_) | SessionEvent::ReasoningDelta(_) => true,
        SessionEvent::SubagentChild { ev, .. } => {
            matches!(ev.as_ref(), SessionEvent::TextDelta(_) | SessionEvent::ReasoningDelta(_))
        }
        _ => false,
    }
}

/// Forward a SessionEvent to the UI channel with backpressure-aware dropping.
/// Droppable streaming deltas are discarded when the channel has <=
/// DELTA_MIN_CAPACITY free slots (final text is always rebuilt from the store
/// on TurnDone). Non-delta lifecycle events always get through via try_send.
fn forward_event(tx: &mpsc::Sender<UiEvent>, sev: SessionEvent) {
    if is_droppable_delta(&sev) && tx.capacity() <= DELTA_MIN_CAPACITY {
        return; // drop delta — final text rebuilt from store on TurnDone
    }
    let _ = tx.try_send(UiEvent::Session(sev));
}

/// Fire-and-forget persist a parent-session event to the store so web/SSE
/// clients can replay sessions driven by the TUI. Awaited (not fire-and-
/// forget) so the event is durable before the worker proceeds — no loss on
/// immediate exit. Used by non-run arms (e.g. SwitchAgent) where no flusher
/// is active.
async fn persist_event(store: &Option<Arc<dyn Store>>, session_id: &str, sev: &SessionEvent) {
    if let Some(store) = store {
        let rec = SessionEventRecord {
            session_id: session_id.to_string(),
            kind: sev.coarse_kind(),
            payload: sev.sse_data(),
            ts: now_ms(),
            seq: None,
            sse_kind: Some(sev.sse_kind().to_string()),
        };
        let _ = store.append_event(&rec).await;
    }
}

/// Process one UI command against a session. Returns `true` when the worker
/// loop should break (Quit).
pub async fn process_cmd(
    cmd: UiCmd,
    sess: &mut SessionState,
    evt_tx: &mpsc::Sender<UiEvent>,
) -> bool {
    match cmd {
        UiCmd::Prompt(prompt) => {
            let tx = evt_tx.clone();
            let (sink, flusher) = spawn_event_flusher(sess.store.clone(), sess.id.clone());
            let sink_for_run = sink.clone();
            let res = run_session(sess, prompt, move |sev| {
                let _ = sink_for_run.push(&sev);
                forward_event(&tx, sev);
            })
            .await;
            if let Err(e) = res {
                let ev = SessionEvent::Error(format!("{e:#}"));
                let _ = sink.push(&ev);
                forward_event(evt_tx, ev);
            }
            // Drop every sender clone so the flusher's channel closes and it
            // performs a final flush — guaranteeing zero event loss this turn.
            drop(sink);
            let _ = flusher.await;
            let _ = evt_tx.send(UiEvent::TurnDone).await;
        }
        UiCmd::SwitchAgent(name) => {
            if let Some(a) = resolve_agent(&name) {
                sess.agent = a;
                let ev = SessionEvent::AgentSwitch(name);
                persist_event(&sess.store, &sess.id, &ev).await;
                forward_event(evt_tx, ev);
            }
        }
        UiCmd::SwitchAndStart(name, extra) => {
            let (sink, flusher) = spawn_event_flusher(sess.store.clone(), sess.id.clone());
            if let Some(a) = resolve_agent(&name) {
                sess.agent = a;
                let ev = SessionEvent::AgentSwitch(name);
                let _ = sink.push(&ev);
                forward_event(evt_tx, ev);
            }
            // Plan→act handoff: clear the transcript so the act agent starts
            // from only the final plan, not the full read-only planning noise.
            // Mirrors compaction — in-memory mutation + TranscriptReset so the
            // UI rebuilds clean; the append-only store keeps the raw history.
            if let Some(plan_display) = opencoder_session::plan_handoff::handoff(sess, &extra) {
                // Persist the handoff boundary so resume reconstructs the
                // focused post-handoff transcript (mirrors compaction).
                if let Some(store) = &sess.store {
                    let _ = store
                        .update_session(
                            &sess.id,
                            &opencoder_store::SessionPatch {
                                handoff_seq: sess.handoff_seq,
                                handoff_plan: sess.handoff_plan.clone(),
                                updated_at: Some(now_ms()),
                                ..Default::default()
                            },
                        )
                        .await;
                }
                let ev = SessionEvent::TranscriptReset(sess.messages.clone());
                let _ = sink.push(&ev);
                forward_event(evt_tx, ev);
                let ev2 = SessionEvent::PlanHandoff(plan_display);
                let _ = sink.push(&ev2);
                forward_event(evt_tx, ev2);
            }
            sess.set_skill(None);
            let tx = evt_tx.clone();
            let sink_for_run = sink.clone();
            let res = run_session(sess, String::new(), move |sev| {
                let _ = sink_for_run.push(&sev);
                forward_event(&tx, sev);
            })
            .await;
            if let Err(e) = res {
                let ev = SessionEvent::Error(format!("{e:#}"));
                let _ = sink.push(&ev);
                forward_event(evt_tx, ev);
            }
            drop(sink);
            let _ = flusher.await;
            let _ = evt_tx.send(UiEvent::TurnDone).await;
        }
        UiCmd::Compact => {
            let registry = opencoder_session::tools::registry();
            let (sink, flusher) = spawn_event_flusher(sess.store.clone(), sess.id.clone());
            // Scope the emit closure so its sender clone is dropped before we
            // drop the last sender + await the flusher (final flush).
            let outcome = {
                let tx = evt_tx.clone();
                let sink_for_emit = sink.clone();
                let mut emit = move |sev: SessionEvent| {
                    let _ = sink_for_emit.push(&sev);
                    forward_event(&tx, sev);
                };
                opencoder_session::compaction::compact(sess, &registry, &mut emit).await
            };
            match outcome {
                Ok(Some(summary)) => {
                    let ev = SessionEvent::TranscriptReset(sess.messages.clone());
                    let _ = sink.push(&ev);
                    forward_event(evt_tx, ev);
                    let ev2 = SessionEvent::Compaction(summary);
                    let _ = sink.push(&ev2);
                    forward_event(evt_tx, ev2);
                }
                Ok(None) => {}
                Err(e) => {
                    let ev = SessionEvent::Error(format!("compaction failed: {e:#}"));
                    let _ = sink.push(&ev);
                    forward_event(evt_tx, ev);
                }
            }
            drop(sink);
            let _ = flusher.await;
            let _ = evt_tx.send(UiEvent::TurnDone).await;
        }
        UiCmd::SetSkill(body) => {
            sess.set_skill(body);
        }
        UiCmd::ReloadConfig(new_cfg) => {
            if let Ok(ep) = new_cfg.resolve_endpoint() {
                if let Ok(new_client) = ChatClient::new(&ep.base_url, &ep.api_key, &ep.headers, new_cfg.network.proxy.as_deref()) {
                    sess.apply_config_reload(*new_cfg, Arc::new(new_client));
                }
            }
        }
        UiCmd::ResetCancel(c) => {
            sess.cancel = Some(c);
        }
        UiCmd::Quit => return true,
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[test]
    fn gate_compact_runs_when_idle() {
        assert_eq!(gate_compact(false), CompactGate::Run);
    }

    #[test]
    fn gate_compact_rejects_when_running() {
        assert_eq!(gate_compact(true), CompactGate::SkipRunning);
    }

    #[test]
    fn gate_clear_all_runs_when_idle() {
        // Idle == all subagents returned → clear is allowed.
        assert_eq!(gate_clear_all(false), ClearAllGate::Run);
    }

    #[test]
    fn gate_clear_all_rejects_when_running() {
        // A turn/subagent in flight must not be cleared (child session live).
        assert_eq!(gate_clear_all(true), ClearAllGate::SkipRunning);
    }

    // Regression guard for F1: after a `/task` session switch the loop's active
    // cancellation token must be the NEW session's token, so double-Esc still
    // interrupts the live session. If `rebind_session` stops reassigning
    // `cancel`, this test fails.
    #[test]
    fn rebind_session_swaps_the_active_cancel_token() {
        // Initial loop state bound to the first session.
        let (mut cmd_tx, _first_cmd_rx) = mpsc::channel::<UiCmd>(8);
        let (_first_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(8);
        let mut session_id = String::from("s1");
        let first_cancel = CancellationToken::new();
        let mut cancel = first_cancel.clone();

        // `/task` switch produces fresh channels + a brand-new token.
        let (new_cmd_tx, _new_cmd_rx) = mpsc::channel::<UiCmd>(8);
        let (_new_evt_tx, new_evt_rx) = mpsc::channel::<UiEvent>(8);
        let new_cancel = CancellationToken::new();
        let new_cancel_probe = new_cancel.clone();

        rebind_session(
            &mut cmd_tx,
            &mut evt_rx,
            &mut session_id,
            &mut cancel,
            new_cmd_tx,
            new_evt_rx,
            "s2".into(),
            new_cancel,
        );

        cancel.cancel();
        assert!(
            new_cancel_probe.is_cancelled(),
            "active loop token must target the switched session"
        );
        assert!(
            !first_cancel.is_cancelled(),
            "old session token must be orphaned, not the active one"
        );
        assert_eq!(session_id, "s2");
    }

    // Regression guard for the "Esc then can't submit" bug: after a double-Esc
    // abort the session's cancel token is permanently cancelled. The loop
    // recovers by sending `ResetCancel(fresh)` before the next turn. This test
    // verifies that `process_cmd(ResetCancel)` actually swaps `sess.cancel` for
    // a fresh, uncancelled token — the exact invariant `run_loop` relies on at
    // its top-of-loop `is_cancelled()` check.
    #[tokio::test]
    async fn reset_cancel_replaces_with_fresh_uncancelled_token() {
        use opencoder_core::resolve_agent;
        use opencoder_llm::MockChatClient;

        let (evt_tx, _evt_rx) = mpsc::channel::<UiEvent>(8);
        let agent = resolve_agent("act").expect("act agent");
        let stale = CancellationToken::new();
        stale.cancel();
        let stale_probe = stale.clone();
        let mut sess = SessionState::new(
            "reset-test",
            agent,
            opencoder_core::Config::default(),
            std::sync::Arc::new(MockChatClient::new())
                as std::sync::Arc<dyn opencoder_llm::ChatStream>,
            std::env::temp_dir(),
        )
        .with_cancel(stale);
        assert!(
            sess.cancel.as_ref().unwrap().is_cancelled(),
            "precondition: token cancelled"
        );

        let fresh = CancellationToken::new();
        let fresh_probe = fresh.clone();
        let should_break = process_cmd(UiCmd::ResetCancel(fresh), &mut sess, &evt_tx).await;

        assert!(!should_break, "ResetCancel must not break the worker loop");
        let active = sess.cancel.as_ref().expect("token present after reset");
        assert!(
            !active.is_cancelled(),
            "session token must be uncancelled after reset"
        );
        assert!(
            !fresh_probe.is_cancelled(),
            "the fresh token itself must be uncancelled"
        );
        assert!(
            stale_probe.is_cancelled(),
            "the old token must remain cancelled (not reused)"
        );
    }

    #[test]
    fn forward_event_throttles_delta_preserves_lifecycle() {
        // Channel with small capacity so we can fill it easily.
        let (tx, _rx) = mpsc::channel::<UiEvent>(2 * DELTA_MIN_CAPACITY + 1);

        // Fill the channel to near-capacity (leave <= DELTA_MIN_CAPACITY free).
        for _ in 0..DELTA_MIN_CAPACITY + 1 {
            tx.try_send(UiEvent::TurnDone).unwrap();
        }
        // Now capacity() <= DELTA_MIN_CAPACITY — deltas should be dropped.
        assert!(tx.capacity() <= DELTA_MIN_CAPACITY);

        // TextDelta is droppable — forward_event should silently drop it.
        forward_event(&tx, SessionEvent::TextDelta("x".into()));
        // Capacity unchanged (event was dropped, not enqueued).
        assert!(tx.capacity() <= DELTA_MIN_CAPACITY);

        // SubagentChild wrapping TextDelta is also droppable.
        forward_event(
            &tx,
            SessionEvent::SubagentChild {
                id: "s1".into(),
                ev: Box::new(SessionEvent::TextDelta("y".into())),
            },
        );

        // SubagentStart is a lifecycle event — must always get through.
        forward_event(
            &tx,
            SessionEvent::SubagentStart {
                id: "s1".into(),
                kind: "explore".into(),
                prompt: "p".into(),
                child_session_id: "c1".into(),
            },
        );
        // The SubagentStart should have been enqueued (capacity decreased by 1).
        assert_eq!(tx.capacity(), DELTA_MIN_CAPACITY - 1);

        // SubagentEnd is a lifecycle event — must always get through.
        forward_event(
            &tx,
            SessionEvent::SubagentEnd {
                id: "s1".into(),
                ok: true,
                cancelled: false,
                summary: "done".into(),
            },
        );
        assert_eq!(tx.capacity(), DELTA_MIN_CAPACITY - 2);
    }
}
