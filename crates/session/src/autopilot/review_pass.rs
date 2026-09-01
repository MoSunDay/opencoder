//! One-shot automatic review pass (`autopilot.mode = "review"`): after the
//! initial task completes, activate the review skill, inject a synthetic
//! review prompt, run a single `run_loop`, then clear the skill and finish.
//! Unlike [`crate::autopilot::drive`] there is no ACT/VERIFY loop — exactly
//! one review turn, then control returns.
//!
//! The pass is read-only and agent-agnostic: the runner dispatch layer admits
//! it for ANY primary agent (the reviewer stays on whatever agent ran the
//! initial task — no switch, no fold). `review` is dispatched after a
//! completed run regardless of whether that agent was `act` or `plan`.

use std::collections::HashMap;

use anyhow::Result;
use opencoder_core::{Message, ToolArc};

use super::phases::activate_review_skill;
use super::prompts::review_prompt;
use super::state::ApPhase;
use crate::runner::{new_id, SessionEvent};
use crate::skill_lifecycle::run_loop_one_shot;
use crate::SessionState;

/// The single review iteration, surfaced in the `AutoPilot` event so
/// surfaces render `autopilot: Review #0`. Zero-based, matching `drive`'s
/// `ApState::iteration` which starts at 0 (and `should_stop`'s
/// `iteration + 1 >= max` cap arithmetic that assumes it).
const REVIEW_ITERATION: u32 = 0;

/// Run the one-shot review pass. Emits `AutoPilot(Review, 0)` before the
/// turn and a terminal `Done` after it — the same end marker `drive` uses,
/// so surfaces need no mode-specific completion handling.
pub async fn review_pass(
    session: &mut SessionState,
    registry: &HashMap<String, ToolArc>,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) -> Result<()> {
    // Pre-flight cancel check, mirroring drive's loop-top guard: a cancel
    // tripped before the pass starts (e.g. during the initial task's final
    // LLM call) must not burn a review turn or leave the session with a
    // residual synthetic prompt. Zero pass side effects — no AutoPilot
    // marker, no injected message — just the same terminal bookkeeping
    // (skill clear + Done) drive's cancel path uses, so surfaces see a
    // uniform end marker either way.
    if super::is_cancelled(session) {
        super::clear_injected_skill(session).await;
        on_event(SessionEvent::Done);
        return Ok(());
    }
    on_event(SessionEvent::AutoPilot {
        phase: ApPhase::Review,
        iteration: REVIEW_ITERATION,
    });
    // No agent switch: the reviewer stays on whatever agent completed the
    // initial task (review is read-only, so act and plan both qualify).
    activate_review_skill(session);
    let goal = super::extract_goal(session);
    let mut msg = Message::user(new_id(), review_prompt(&goal));
    msg.synthetic = true;
    session.record(msg).await;
    let run = run_loop_one_shot(session, registry, on_event, false).await;
    // One-shot: clear the review skill and emit the uniform end marker on
    // BOTH outcomes — an LLM failure mid-review (e.g. 429 exhaustion) must
    // not leave the system-injected skill stuck on the session, in memory
    // or persisted (a resume would otherwise resurrect it). Mirrors the
    // drive() error path, which runs the same terminal bookkeeping before
    // propagating. A cancel mid-review surfaces via run_loop's own
    // interruption handling.
    super::clear_injected_skill(session).await;
    on_event(SessionEvent::Done);
    run
}
