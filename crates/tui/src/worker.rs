//! Background worker command processing — shared by the main worker and the
//! `/task`-spawned worker to avoid duplicate match arms.

use std::sync::Arc;

use opencoder_core::{message::now_ms, resolve_agent, Config};
use opencoder_llm::ChatClient;
use opencoder_session::{run as run_session, SessionEvent, SessionState};
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

/// Fire-and-forget persist a parent-session event to the store so web/SSE
/// clients can replay sessions driven by the TUI. Mirrors the web drain path.
fn persist_event(store: &Option<Arc<dyn Store>>, session_id: &str, sev: &SessionEvent) {
    if let Some(store) = store {
        let rec = SessionEventRecord {
            session_id: session_id.to_string(),
            kind: sev.coarse_kind(),
            payload: sev.sse_data(),
            ts: now_ms(),
            seq: None,
            sse_kind: Some(sev.sse_kind().to_string()),
        };
        let store = Arc::clone(store);
        tokio::spawn(async move {
            let _ = store.append_event(&rec).await;
        });
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
            let store = sess.store.clone();
            let sid = sess.id.clone();
            let res = run_session(sess, prompt, move |sev| {
                persist_event(&store, &sid, &sev);
                let _ = tx.try_send(UiEvent::Session(sev));
            })
            .await;
            if let Err(e) = res {
                let ev = SessionEvent::Error(format!("{e:#}"));
                persist_event(&sess.store, &sess.id, &ev);
                let _ = evt_tx.try_send(UiEvent::Session(ev));
            }
            let _ = evt_tx.send(UiEvent::TurnDone).await;
        }
        UiCmd::SwitchAgent(name) => {
            if let Some(a) = resolve_agent(&name) {
                sess.agent = a;
                let ev = SessionEvent::AgentSwitch(name);
                persist_event(&sess.store, &sess.id, &ev);
                let _ = evt_tx.try_send(UiEvent::Session(ev));
            }
        }
        UiCmd::SwitchAndStart(name, extra) => {
            if let Some(a) = resolve_agent(&name) {
                sess.agent = a;
                let ev = SessionEvent::AgentSwitch(name);
                persist_event(&sess.store, &sess.id, &ev);
                let _ = evt_tx.try_send(UiEvent::Session(ev));
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
                persist_event(&sess.store, &sess.id, &ev);
                let _ = evt_tx.try_send(UiEvent::Session(ev));
                let ev2 = SessionEvent::PlanHandoff(plan_display);
                persist_event(&sess.store, &sess.id, &ev2);
                let _ = evt_tx.try_send(UiEvent::Session(ev2));
            }
            let tx = evt_tx.clone();
            let store = sess.store.clone();
            let sid = sess.id.clone();
            let res = run_session(sess, String::new(), move |sev| {
                persist_event(&store, &sid, &sev);
                let _ = tx.try_send(UiEvent::Session(sev));
            })
            .await;
            if let Err(e) = res {
                let ev = SessionEvent::Error(format!("{e:#}"));
                persist_event(&sess.store, &sess.id, &ev);
                let _ = evt_tx.try_send(UiEvent::Session(ev));
            }
            let _ = evt_tx.send(UiEvent::TurnDone).await;
        }
        UiCmd::Compact => {
            let registry = opencoder_session::tools::registry();
            let tx = evt_tx.clone();
            let store = sess.store.clone();
            let sid = sess.id.clone();
            let mut emit = move |sev: SessionEvent| {
                persist_event(&store, &sid, &sev);
                let _ = tx.try_send(UiEvent::Session(sev));
            };
            match opencoder_session::compaction::compact(sess, &registry, &mut emit).await {
                Ok(Some(summary)) => {
                    let ev = SessionEvent::TranscriptReset(sess.messages.clone());
                    persist_event(&sess.store, &sess.id, &ev);
                    let _ = evt_tx.try_send(UiEvent::Session(ev));
                    let ev2 = SessionEvent::Compaction(summary);
                    persist_event(&sess.store, &sess.id, &ev2);
                    let _ = evt_tx.try_send(UiEvent::Session(ev2));
                }
                Ok(None) => {}
                Err(e) => {
                    let ev = SessionEvent::Error(format!("compaction failed: {e:#}"));
                    persist_event(&sess.store, &sess.id, &ev);
                    let _ = evt_tx.try_send(UiEvent::Session(ev));
                }
            }
            let _ = evt_tx.send(UiEvent::TurnDone).await;
        }
        UiCmd::SetSkill(body) => {
            sess.set_skill(body);
        }
        UiCmd::ReloadConfig(new_cfg) => {
            let api_key = new_cfg.api_key().unwrap_or_default();
            if let Ok(new_client) = ChatClient::new(&new_cfg.provider.base_url, &api_key, new_cfg.network.proxy.as_deref()) {
                sess.apply_config_reload(*new_cfg, Arc::new(new_client));
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
}
