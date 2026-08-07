//! Autopilot: a self-driving PLAN -> ACT -> VERIFY loop.
//!
//! When `config.autopilot.enabled` is on, the session runner hands control to
//! [`drive`] after the initial task. Each iteration:
//!
//! - **PLAN** — switch to the plan agent, inject a continuation prompt, run one
//!   loop. Plan turns stay in the transcript (legitimate work record).
//! - **ACT** — reset the transcript via plan→act handoff (ACT sees only the
//!   plan output as its execution instruction), switch to the act agent, run
//!   one loop. Inject an execute prompt only as a fallback when no handoff
//!   plan is found.
//! - **VERIFY** — an isolated *shadow* one-shot: it clones the current
//!   transcript into a throwaway snapshot, asks a small model "is the goal
//!   fully achieved?", parses a single yes/no, then discards the snapshot.
//!   Nothing is recorded or persisted — the main transcript is never polluted
//!   by the judgement exchange.
//!
//! The loop stops when VERIFY says "yes" (complete), retries exhaust on
//! malformed verdicts (aborted), the session is cancelled, or `max_iterations`
//! is hit. The existing doom-loop / tool-failure / cancel guards inside
//! `run_loop` still terminate individual phase runs.

mod decision;
mod phases;
mod prompts;
pub mod state;
mod verify;

pub use decision::{parse_verdict, should_stop};
pub use state::{ApOutcome, ApPhase, ApState, VerifyVerdict};
pub use verify::verify;

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use anyhow::Result;
use opencoder_core::{Role, ToolArc};

use crate::autopilot::phases::{run_act_phase, run_plan_phase};
use crate::runner::SessionEvent;
use crate::SessionState;

/// `true` when the session's cancellation token has been tripped.
fn is_cancelled(session: &SessionState) -> bool {
    session
        .cancel
        .as_ref()
        .map(|c| c.is_cancelled())
        .unwrap_or(false)
}

/// Extract the goal from the first real (non-synthetic) user message. This
/// anchors the VERIFY judgement to the original intent across all iterations.
fn extract_goal(session: &SessionState) -> String {
    session
        .messages
        .iter()
        .find(|m| m.role == Role::User && !m.synthetic)
        .map(|m| m.text())
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "the user's original request".to_string())
}

/// Drive the PLAN -> ACT -> VERIFY loop until a terminal outcome is reached.
///
/// `session` is mutated (agent switches + recorded phase prompts) but the
/// VERIFY exchange is fully isolated — see [`verify::verify`].
pub async fn drive(
    session: &mut SessionState,
    registry: &HashMap<String, ToolArc>,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) -> Result<ApOutcome> {
    // Copy the Copy-type config knobs out so we don't hold an immutable borrow
    // of `session.config` across the mutable phase calls. Degenerate values are
    // clamped: 0 iterations would silently spin, 0 verify retries would never
    // judge at all.
    let max_iterations = session.config.autopilot.max_iterations.max(1);
    let verify_retries = session.config.autopilot.verify_retries.max(1);
    let mut state = ApState::new(extract_goal(session));

    loop {
        if is_cancelled(session) {
            return finish(session, on_event, ApOutcome::Cancelled);
        }
        if state.iteration >= max_iterations {
            return finish(session, on_event, ApOutcome::MaxIterations);
        }
        on_event(SessionEvent::AutoPilot {
            phase: ApPhase::Plan,
            iteration: state.iteration,
        });
        // A phase error still runs the terminal bookkeeping (skill cleared +
        // Done) so the next user turn doesn't inherit the review skill, then
        // the error propagates to the caller unchanged.
        if let Err(e) = run_plan_phase(session, registry, on_event, &state).await {
            finish(
                session,
                on_event,
                ApOutcome::Aborted(format!("plan phase failed: {e:#}")),
            )?;
            return Err(e);
        }

        if is_cancelled(session) {
            return finish(session, on_event, ApOutcome::Cancelled);
        }
        on_event(SessionEvent::AutoPilot {
            phase: ApPhase::Act,
            iteration: state.iteration,
        });
        if let Err(e) = run_act_phase(session, registry, on_event).await {
            finish(
                session,
                on_event,
                ApOutcome::Aborted(format!("act phase failed: {e:#}")),
            )?;
            return Err(e);
        }

        // A cancel during ACT (run_loop broke with Status("interrupted")) must
        // not burn a VERIFY call.
        if is_cancelled(session) {
            return finish(session, on_event, ApOutcome::Cancelled);
        }
        on_event(SessionEvent::AutoPilot {
            phase: ApPhase::Verify,
            iteration: state.iteration,
        });
        // VERIFY is deliberately NOT cancel-checked: it is a short one-shot
        // (max_tokens=8). A cancel tripped mid-judge surfaces at the next
        // loop-top check; the runner-level cancel still stops the outer run.
        let verdict = verify(session, &state, verify_retries).await;
        match should_stop(verdict, state.iteration, max_iterations) {
            Some(outcome) => return finish(session, on_event, outcome),
            None => state.iteration += 1, // MoreWork, under cap → loop again
        }
    }
}

/// Terminal bookkeeping for every outcome: clear the active skill, emit a
/// final `Done` so surfaces get a uniform end-of-autopilot marker, and return
/// the outcome. `should_stop` never yields `Cancelled` — that path is handled
/// by the explicit checks above.
fn finish(
    session: &mut SessionState,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
    outcome: ApOutcome,
) -> Result<ApOutcome> {
    session.set_skill(None);
    on_event(SessionEvent::Done);
    Ok(outcome)
}
