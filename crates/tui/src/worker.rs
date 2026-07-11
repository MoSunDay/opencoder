//! Background worker command processing — shared by the main worker and the
//! `/task`-spawned worker to avoid duplicate match arms.

use std::sync::Arc;

use opencode_core::{resolve_agent, Config};
use opencode_llm::ChatClient;
use opencode_session::{run as run_session, SessionEvent, SessionState};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub enum UiCmd {
    Prompt(String),
    SwitchAgent(String),
    /// Switch agent then immediately start a turn without recording a new user
    /// message. Used for the plan->act manual transition: the system prompt
    /// changes to act and the model reads the plan from conversation history.
    SwitchAndStart(String),
    /// Manually trigger conversation compaction.
    Compact,
    SetSkill(Option<String>),
    /// Hot-reload config at the next turn boundary. Sent by the `/model` menu.
    ReloadConfig(Config),
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
            let res = run_session(sess, prompt, move |sev| {
                let _ = tx.try_send(UiEvent::Session(sev));
            })
            .await;
            if let Err(e) = res {
                let _ = evt_tx.try_send(UiEvent::Session(SessionEvent::Error(format!("{e:#}"))));
            }
            let _ = evt_tx.try_send(UiEvent::TurnDone);
        }
        UiCmd::SwitchAgent(name) => {
            if let Some(a) = resolve_agent(&name) {
                sess.agent = a;
                let _ = evt_tx.try_send(UiEvent::Session(SessionEvent::AgentSwitch(name)));
            }
        }
        UiCmd::SwitchAndStart(name) => {
            if let Some(a) = resolve_agent(&name) {
                sess.agent = a;
                let _ = evt_tx.try_send(UiEvent::Session(SessionEvent::AgentSwitch(name)));
            }
            let tx = evt_tx.clone();
            let res = run_session(sess, String::new(), move |sev| {
                let _ = tx.try_send(UiEvent::Session(sev));
            })
            .await;
            if let Err(e) = res {
                let _ = evt_tx.try_send(UiEvent::Session(SessionEvent::Error(format!("{e:#}"))));
            }
            let _ = evt_tx.try_send(UiEvent::TurnDone);
        }
        UiCmd::Compact => {
            let registry = opencode_session::tools::registry();
            match opencode_session::compaction::compact(sess, &registry).await {
                Ok(Some(summary)) => {
                    let _ = evt_tx.try_send(UiEvent::Session(SessionEvent::TranscriptReset(
                        sess.messages.clone(),
                    )));
                    let _ = evt_tx.try_send(UiEvent::Session(SessionEvent::Compaction(summary)));
                }
                Ok(None) => {}
                Err(e) => {
                    let _ = evt_tx.try_send(UiEvent::Session(SessionEvent::Error(format!(
                        "compaction failed: {e:#}"
                    ))));
                }
            }
            let _ = evt_tx.try_send(UiEvent::TurnDone);
        }
        UiCmd::SetSkill(body) => {
            sess.skill_prompt = body;
        }
        UiCmd::ReloadConfig(new_cfg) => {
            let api_key = new_cfg.api_key().unwrap_or_default();
            if let Ok(new_client) = ChatClient::new(&new_cfg.provider.base_url, &api_key) {
                sess.apply_config_reload(new_cfg, Arc::new(new_client));
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
        use opencode_core::resolve_agent;
        use opencode_llm::MockChatClient;

        let (evt_tx, _evt_rx) = mpsc::channel::<UiEvent>(8);
        let agent = resolve_agent("act").expect("act agent");
        let stale = CancellationToken::new();
        stale.cancel();
        let stale_probe = stale.clone();
        let mut sess = SessionState::new(
            "reset-test",
            agent,
            opencode_core::Config::default(),
            std::sync::Arc::new(MockChatClient::new())
                as std::sync::Arc<dyn opencode_llm::ChatStream>,
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
