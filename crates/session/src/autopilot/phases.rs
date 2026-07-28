//! Phase runners. Each takes a mutable borrow of the session (it records
//! messages + may switch the agent) and drives one `run_loop`.

use std::collections::HashMap;

use anyhow::Result;
use opencoder_core::{resolve_agent, Message, ToolArc};

use crate::autopilot::prompts::{continuation_prompt, execute_prompt};
use crate::autopilot::state::ApState;
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

/// Best-effort: resolve a configured skill name to its body and activate it.
/// Skills are discovered from `~/.opencoder/skills`; a missing skill is a no-op.
fn maybe_activate_skill(session: &SessionState) {
    if let Some(name) = &session.config.autopilot.skill {
        let body = opencoder_core::skill::discover()
            .into_iter()
            .find(|s| &s.name == name)
            .map(|s| s.body);
        session.set_skill(body);
    }
}

/// PLAN phase: switch to the plan agent, (optionally) activate the configured
/// skill, inject the continuation prompt, and run one loop.
pub async fn run_plan_phase(
    session: &mut SessionState,
    registry: &HashMap<String, ToolArc>,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
    state: &ApState,
) -> Result<()> {
    switch_agent(session, "plan", on_event);
    maybe_activate_skill(session);
    let mut msg = Message::user(new_id(), continuation_prompt(&state.goal));
    msg.synthetic = true;
    session.record(msg).await;
    run_loop(session, registry, on_event).await
}

/// ACT phase: switch to the act agent, inject the execute prompt, and run one
/// loop. Context is carried over from PLAN (no handoff reset) so VERIFY can
/// inspect the complete work record.
pub async fn run_act_phase(
    session: &mut SessionState,
    registry: &HashMap<String, ToolArc>,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) -> Result<()> {
    switch_agent(session, "act", on_event);
    let mut msg = Message::user(new_id(), execute_prompt());
    msg.synthetic = true;
    session.record(msg).await;
    run_loop(session, registry, on_event).await
}
