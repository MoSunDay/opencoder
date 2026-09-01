//! Phase runners. Each takes a mutable borrow of the session (it records
//! messages + may switch the agent) and drives one `run_loop`.

use std::collections::HashMap;

use anyhow::Result;
use opencoder_core::message::now_ms;
use opencoder_core::{resolve_agent, Message, ToolArc};
use opencoder_store::SessionPatch;

use crate::autopilot::prompts::{continuation_prompt, execute_prompt};
use crate::autopilot::state::ApState;
use crate::handoff;
use crate::runner::{new_id, SessionEvent};
use crate::skill_lifecycle::run_loop_one_shot;
use crate::SessionState;

/// Switch the active agent, emitting an `AgentSwitch` event so surfaces stay
/// in sync. Falls back to the current agent if the requested name is unknown.
pub(super) fn switch_agent(
    session: &mut SessionState,
    name: &str,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) {
    if let Some(agent) = resolve_agent(name) {
        session.agent = agent;
    }
    on_event(SessionEvent::AgentSwitch(session.agent.name.clone()));
}

/// Hardcode the review skill for the review pass. Always discovers the
/// `"review"` skill from `~/.opencoder/skills`; a missing skill body is a
/// no-op (skill set to `None`).
pub(super) fn activate_review_skill(session: &SessionState) {
    activate_skill(session, "review");
}

/// Activate a discovered skill by name for this session. A missing skill is a
/// no-op (`set_skill(None)`-equivalent: nothing is injected). The body is
/// wrapped with its source path so the model can resolve skill-relative assets.
pub(super) fn activate_skill(session: &SessionState, name: &str) {
    let body = opencoder_core::skill::discover()
        .into_iter()
        .find(|s| s.name == name)
        .map(|s| opencoder_core::body_with_source(&s));
    session.set_skill(body);
}

/// PLAN phase: switch to the plan agent (read-only explorer), activate the
/// task-plan skill (which unlocks the latent `question` tool), inject the
/// continuation prompt, and run one loop.
pub async fn run_plan_phase(
    session: &mut SessionState,
    registry: &HashMap<String, ToolArc>,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
    state: &ApState,
) -> Result<()> {
    switch_agent(session, "plan", on_event);
    activate_skill(session, "task-plan");
    let mut msg = Message::user(new_id(), continuation_prompt(&state.goal));
    msg.synthetic = true;
    session.record(msg).await;
    run_loop_one_shot(session, registry, on_event, false).await
}

/// ACT phase: reset the transcript via execution handoff so ACT only sees the
/// planning brief as its sole execution instruction, then run one loop. If the
/// handoff cannot find a brief (no assistant text), fall back to injecting an
/// explicit execute prompt.
pub async fn run_act_phase(
    session: &mut SessionState,
    registry: &HashMap<String, ToolArc>,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) -> Result<()> {
    if handoff::reset_to_directive(session, "").is_some() {
        // Persist the handoff boundary so resume can reconstruct the focused
        // transcript. Best-effort: non-fatal if the store is absent.
        if let Some(store) = &session.store {
            let _ = store
                .update_session(
                    &session.id,
                    &SessionPatch {
                        handoff_seq: session.handoff_seq,
                        handoff_plan: session.handoff_plan.clone(),
                        clear_summary: true,
                        clear_skill: true,
                        updated_at: Some(now_ms()),
                        ..Default::default()
                    },
                )
                .await;
        }
        on_event(SessionEvent::TranscriptReset(session.messages.clone()));
        session.set_skill(None);
        switch_agent(session, "act", on_event);
        // The handoff message already carries execution directives
        // (HANDOFF_PREFIX), so no separate execute_prompt is injected.
        run_loop_one_shot(session, registry, on_event, false).await
    } else {
        // No plan found (fallback execute prompt): the handoff branch above
        // persists its clear via the combined patch; this branch must clear
        // durably too, or a crash/resume mid-ACT would resurrect the
        // system-injected review skill from the `sessions.skill` column.
        super::clear_injected_skill(session, on_event).await;
        switch_agent(session, "act", on_event);
        let mut msg = Message::user(new_id(), execute_prompt());
        msg.synthetic = true;
        session.record(msg).await;
        run_loop_one_shot(session, registry, on_event, false).await
    }
}
