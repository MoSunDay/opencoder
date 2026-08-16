//! Phase runners. Each takes a mutable borrow of the session (it records
//! messages + may switch the agent) and drives one `run_loop`.

use std::collections::HashMap;

use anyhow::Result;
use opencoder_core::message::now_ms;
use opencoder_core::{resolve_agent, Message, ToolArc};
use opencoder_store::SessionPatch;

use crate::autopilot::prompts::{continuation_prompt, execute_prompt};
use crate::autopilot::state::ApState;
use crate::plan_handoff;
use crate::runner::{new_id, run_loop, SessionEvent};
use crate::SessionState;

/// Switch the active agent, emitting an `AgentSwitch` event so surfaces stay
/// in sync. Falls back to the current agent if the requested name is unknown.
fn switch_agent(
    session: &mut SessionState,
    name: &str,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) {
    if let Some(agent) = resolve_agent(name) {
        session.agent = agent;
    }
    on_event(SessionEvent::AgentSwitch(session.agent.name.clone()));
}

/// Hardcode the review skill for the PLAN phase. Always discovers the
/// `"review"` skill from `~/.opencoder/skills`; a missing skill body is a
/// no-op (skill set to `None`).
fn activate_review_skill(session: &SessionState) {
    let body = opencoder_core::skill::discover()
        .into_iter()
        .find(|s| s.name == "review")
        .map(|s| opencoder_core::body_with_source(&s));
    session.set_skill(body);
}

/// PLAN phase: switch to the plan agent, activate the review skill, inject the
/// continuation prompt, and run one loop.
pub async fn run_plan_phase(
    session: &mut SessionState,
    registry: &HashMap<String, ToolArc>,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
    state: &ApState,
) -> Result<()> {
    switch_agent(session, "plan", on_event);
    activate_review_skill(session);
    let mut msg = Message::user(new_id(), continuation_prompt(&state.goal));
    msg.synthetic = true;
    session.record(msg).await;
    run_loop(session, registry, on_event, false).await
}

/// ACT phase: reset the transcript via plan→act handoff so ACT only sees the
/// review output as its sole execution instruction, then run one loop. If the
/// handoff cannot find a plan (no assistant text), fall back to injecting an
/// explicit execute prompt.
pub async fn run_act_phase(
    session: &mut SessionState,
    registry: &HashMap<String, ToolArc>,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) -> Result<()> {
    if plan_handoff::handoff(session, "").is_some() {
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
        run_loop(session, registry, on_event, false).await
    } else {
        session.set_skill(None);
        switch_agent(session, "act", on_event);
        let mut msg = Message::user(new_id(), execute_prompt());
        msg.synthetic = true;
        session.record(msg).await;
        run_loop(session, registry, on_event, false).await
    }
}
