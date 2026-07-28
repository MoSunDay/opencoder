//! Autopilot: a self-driving PLAN -> ACT -> VERIFY loop.
//!
//! When `config.autopilot.enabled` is on, the session runner hands control to
//! [`drive`] after the initial task. Each iteration:
//!
//! - **PLAN** — switch to the plan agent, inject a continuation prompt, run one
//!   loop. Plan turns stay in the transcript (legitimate work record).
//! - **ACT** — switch to the act agent (context carried over, no reset), inject
//!   an execute prompt, run one loop.
//! - **VERIFY** — an isolated *shadow* one-shot: it clones the current
//!   transcript into a throwaway snapshot, asks a small model "is more work
//!   needed?", parses a single yes/no, then discards the snapshot. Nothing is
//!   recorded or persisted — the main transcript is never polluted by the
//!   judgement exchange.
//!
//! The loop stops when VERIFY says "no" (complete), retries exhaust on
//! malformed verdicts (aborted), or `max_iterations` is hit. The existing
//! doom-loop / tool-failure / cancel guards inside `run_loop` still terminate
//! individual phase runs.

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
    // of `session.config` across the mutable phase calls.
    let max_iterations = session.config.autopilot.max_iterations;
    let verify_retries = session.config.autopilot.verify_retries;
    let mut state = ApState::new(extract_goal(session));

    while state.iteration < max_iterations {
        if is_cancelled(session) {
            break;
        }
        on_event(SessionEvent::AutoPilot {
            phase: ApPhase::Plan,
            iteration: state.iteration,
        });
        run_plan_phase(session, registry, on_event, &state).await?;

        if is_cancelled(session) {
            break;
        }
        on_event(SessionEvent::AutoPilot {
            phase: ApPhase::Act,
            iteration: state.iteration,
        });
        run_act_phase(session, registry, on_event).await?;

        on_event(SessionEvent::AutoPilot {
            phase: ApPhase::Verify,
            iteration: state.iteration,
        });
        let verdict = verify(session, &state, verify_retries).await;
        match should_stop(verdict, state.iteration, max_iterations) {
            Some(ApOutcome::Complete) => {
                on_event(SessionEvent::Done);
                return Ok(ApOutcome::Complete);
            }
            Some(other) => return Ok(other),
            None => state.iteration += 1, // MoreWork, under cap → loop again
        }
    }
    Ok(ApOutcome::MaxIterations)
}
