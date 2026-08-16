//! One-shot automatic review pass (`autopilot.mode = "review"`): after the
//! initial task completes, switch to the plan agent, activate the review
//! skill, inject a synthetic review prompt, run a single `run_loop`, then
//! clear the skill and finish. Unlike [`crate::autopilot::drive`] there is no
//! ACT/VERIFY loop — exactly one review turn, then control returns.

use std::collections::HashMap;

use anyhow::Result;
use opencoder_core::{Message, ToolArc};

use super::phases::{activate_review_skill, switch_agent};
use super::prompts::review_prompt;
use super::state::ApPhase;
use crate::runner::{new_id, run_loop, SessionEvent};
use crate::SessionState;

/// The single review iteration, surfaced in the `AutoPilot` event so
/// surfaces render `autopilot: Review (iteration 1)`.
const REVIEW_ITERATION: u32 = 1;

/// Run the one-shot review pass. Emits `AutoPilot(Review, 1)` before the
/// turn and a terminal `Done` after it — the same end marker `drive` uses,
/// so surfaces need no mode-specific completion handling.
pub async fn review_pass(
    session: &mut SessionState,
    registry: &HashMap<String, ToolArc>,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) -> Result<()> {
    on_event(SessionEvent::AutoPilot {
        phase: ApPhase::Review,
        iteration: REVIEW_ITERATION,
    });
    switch_agent(session, "plan", on_event);
    activate_review_skill(session);
    let goal = super::extract_goal(session);
    let mut msg = Message::user(new_id(), review_prompt(&goal));
    msg.synthetic = true;
    session.record(msg).await;
    run_loop(session, registry, on_event, false).await?;
    // One-shot: clear the review skill and emit the uniform end marker. A
    // cancel mid-review surfaces via run_loop's own interruption handling.
    session.set_skill(None);
    on_event(SessionEvent::Done);
    Ok(())
}
