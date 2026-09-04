//! Queueable control commands: `/act`, `/plan`, `/act_clear_context`
//! (`/clear_context` is the accepted legacy alias).
//!
//! These slash commands switch the runtime agent and/or clear the transcript.
//! They take effect *immediately* when consumed by the drain loop. Pure mode
//! switches do not consume an LLM turn; a clear with preserved context does,
//! so act can continue from the seed or execute the preserved plan. Public UI
//! admission rejects them while a run is active; the runner parser remains so
//! idle submissions and persisted/internal recovery inputs stay deterministic:
//!
//! ```text
//! queue: [/plan] -> [review skill] -> [/act]
//! drain: switch->plan (no turn) . run "review" . switch->act (no turn)
//! ```
//!
//! Integration points (all in [`crate::runner`]):
//! - **Idle short-circuit** ([`run_with_registry`]): when the idle prompt is a
//!   control command, apply it and return without entering [`run_loop`].
//! - **Queue intercept** (run_loop idle boundary): drain consecutive control
//!   commands without an LLM turn until a real prompt arrives.
//! - **Steer intercept** (run_loop turn boundary, defensive): a steered control
//!   command is applied immediately instead of being recorded as user text.

use anyhow::Result;
use opencoder_core::agent::list_agents;
use opencoder_core::{
    builtin_agents, message::now_ms, resolve_agent, AgentKind, AgentMode, ContentBlock, Message,
};
use opencoder_store::SessionPatch;

use crate::runner::new_id;
use crate::runner::SessionEvent;
use crate::SessionState;

/// Sentinel value stored in `handoff_plan` so [`crate::resume`] reconstructs a
/// fresh-start marker (not a directive boundary) after a
/// [`ControlCmd::ClearContext`]. The distinctive ASCII framing guarantees it
/// never collides with real content (no LLM/user output starts with this).
pub(crate) const CLEAR_CONTEXT_SENTINEL: &str = "<<OPENCODER_CLEAR_CONTEXT_MARKER>>";

/// True when a persisted `handoff_plan` is the clear-context sentinel — i.e.
/// the boundary was written by [`ControlCmd::ClearContext`] and preserved
/// nothing. Public so display layers (TUI handoff card, CLI JSON dump) can
/// skip the raw sentinel instead of ever outputting it; the LLM must never
/// see it (resume converts it to [`fresh_start_message`] before rebuilding
/// context).
pub fn is_clear_context_handoff(handoff_plan: &str) -> bool {
    handoff_plan == CLEAR_CONTEXT_SENTINEL
}

/// Marker prefix stored in `handoff_plan` when [`ControlCmd::ClearContext`]
/// preserved the last assistant reply as a continuity seed: the persisted
/// value is this prefix followed by the preserved reply text. Same ASCII
/// framing rationale as [`CLEAR_CONTEXT_SENTINEL`] — it can never collide
/// with real content.
pub(crate) const CLEAR_CONTEXT_SEED_PREFIX: &str = "<<OPENCODER_CLEAR_SEED>>";

/// True when a persisted `handoff_plan` is a clear-context seed boundary —
/// the clear preserved the last assistant reply. Public so display layers
/// (TUI replay, CLI JSON dump) can strip the marker and render the preserved
/// text; the LLM must never see the raw marker (resume converts it back to a
/// [`seed_message`] before rebuilding context).
pub fn is_clear_context_seed(handoff_plan: &str) -> bool {
    handoff_plan.starts_with(CLEAR_CONTEXT_SEED_PREFIX)
}

/// The preserved reply text carried by a clear-context seed boundary (the
/// marker prefix stripped). Only meaningful when [`is_clear_context_seed`]
/// holds.
pub fn clear_seed_text(handoff_plan: &str) -> &str {
    handoff_plan
        .strip_prefix(CLEAR_CONTEXT_SEED_PREFIX)
        .unwrap_or("")
}

/// Marker value persisted in `handoff_plan` for a seed boundary.
fn clear_seed_marker(text: &str) -> String {
    format!("{CLEAR_CONTEXT_SEED_PREFIX}{text}")
}

/// Body of the fresh-start marker message left after a context clear.
const CLEAR_CONTEXT_BODY: &str = "[Context cleared - starting fresh.]";

/// Neutral wrapper for the preserved last say. Deliberately NOT an execution
/// directive ([`crate::handoff`] prefixes the autopilot handoff instead): the
/// preserved text is a plain prior answer ("task done"), not a task, and an
/// execution directive would fabricate a task out of finished work.
const CLEAR_SEED_BODY_PREFIX: &str = "[Context cleared. The previous assistant reply below \
is preserved as continuity context - prior context, not a new instruction.]\n\n";

/// A control command parsed from a slash-command string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlCmd {
    /// Switch the active agent without resetting context. The name may be a
    /// builtin (`act`/`plan` via `/act`//`/plan`) or any resolvable
    /// file-based agent (via `/agent <name>`).
    SwitchAgent(String),
    /// Bare `/agent`: list the available agents (primary builtins plus every
    /// file-based agent card) in an info event. No state change.
    AgentList,
    /// Clear the transcript. From plan mode, the newest plan is preserved as
    /// an execution directive and the session converges to act; this is the
    /// handoff promised by the `/act_` prefix. From act, the newest assistant
    /// reply remains a neutral continuity seed. Only a transcript with no
    /// assistant content collapses to the blank fresh-start marker.
    ClearContext,
}

/// Split a compound prompt into its leading control command and any trailing
/// argument text. Supports `/plan <args>` and `/act <args>` so a single
/// submission like `/plan review` switches agent *and* runs the rest as a
/// prompt in the new agent.
///
/// `/agent <name> [args]` switches to ANY resolvable agent — builtin or
/// file-based (`resolve_agent` decides). The first token after `/agent` is
/// the agent name; anything after it is a compound prompt in the new agent
/// (`/agent plan review` ≡ `/plan review`). A bare `/agent` (no name) parses
/// to [`ControlCmd::AgentList`], which lists the available agents instead of
/// switching.
///
/// The clear-context fold is canonicalized as `/act_clear_context` — the
/// `act` prefix kept explicit so the command reads as the act-agent
/// fold-and-restart it is. It supports compound inputs like
/// `/act_clear_context review` where the trailing text runs as a prompt in
/// the fresh context. The legacy spelling `/clear_context` still parses
/// (mapped to the same command) so already-persisted inputs keep behaving
/// deterministically. From plan mode the clear preserves the plan as an
/// execution directive and converges to act before the continuation turn.
///
/// Returns `None` for anything that is not a control command. The rest text is
/// the trimmed remainder after the command token (for `/agent`, after the
/// agent-name token), or `None` when the input was a bare command with
/// nothing following (e.g. `/act`). Inline `$skill` tokens in the rest are
/// preserved verbatim for downstream resolution.
pub fn split_control_prefix(prompt: &str) -> Option<(ControlCmd, Option<String>)> {
    let trimmed = prompt.trim();
    let mut parts = trimmed.split_whitespace();
    let head = parts.next()?;
    let cmd = match head {
        "/act" => ControlCmd::SwitchAgent("act".into()),
        "/plan" => ControlCmd::SwitchAgent("plan".into()),
        "/act_clear_context" | "/clear_context" => ControlCmd::ClearContext,
        "/agent" => {
            // The token after `/agent` is the target agent name; anything
            // beyond it is a compound prompt (like `/plan review`).
            match parts.next() {
                Some(name) => ControlCmd::SwitchAgent(name.into()),
                None => ControlCmd::AgentList,
            }
        }
        _ => return None,
    };
    // The prompt tail after the command head: for `/agent <name> …` the
    // name token is part of the command, so the tail starts after it.
    let after_head = trimmed.strip_prefix(head).map(str::trim);
    let rest = match (&cmd, head) {
        (ControlCmd::SwitchAgent(name), "/agent") => after_head
            .and_then(|after| after.strip_prefix(name.as_str()))
            .map(str::trim),
        _ => after_head,
    }
    .filter(|s| !s.is_empty())
    .map(str::to_string);
    Some((cmd, rest))
}

/// The transcript echo for a consumed input: for a compound control command
/// the tail (exactly what `record_compound` records as the real user turn),
/// `None` for a bare control command (applied inline — nothing recorded, so
/// nothing to echo), and the input itself for non-control text. Single source
/// of truth for the "slash command never echoes; its compound tail does"
/// contract across the runner events, the TUI transcript and the CLI header.
pub fn consumed_echo_text(input: &str) -> Option<String> {
    match split_control_prefix(input) {
        Some((_, Some(rest))) => Some(rest),
        Some((_, None)) => None,
        None => Some(input.to_string()),
    }
}

/// PARENT session: queue/steer submissions are admitted and the runner applies
/// them at the next idle/turn boundary, which structurally has no turn in
/// flight. Still the admission guard for subagent steers (subagents have no
/// agent concept) and the TUI's subagent-focus gate.
pub fn is_mode_control(prompt: &str) -> bool {
    split_control_prefix(prompt).is_some()
}

/// Parse a user prompt into a control command. Returns `None` for anything
/// that is not `/act`, `/plan`, `/act_clear_context` (or the legacy
/// `/clear_context`); all accept an optional trailing argument. Compound
/// inputs like `/plan review` are recognized as a control command; use
/// [`split_control_prefix`] to also recover the trailing argument.
pub fn parse(prompt: &str) -> Option<ControlCmd> {
    split_control_prefix(prompt).map(|(cmd, _)| cmd)
}

/// Apply a control command to `session`, mutating state, persisting via the
/// store, and emitting the appropriate [`SessionEvent`]s so the UI updates.
/// Never calls the LLM.
pub async fn apply(
    session: &mut SessionState,
    cmd: &ControlCmd,
    on_event: &mut (impl FnMut(SessionEvent) + ?Sized),
) -> Result<()> {
    match cmd {
        ControlCmd::SwitchAgent(name) => {
            match resolve_agent(name) {
                Some(a) => {
                    // Switching to the agent already in charge is a pure no-op:
                    // no persistence write, no AgentSwitch event, no transcript
                    // side effects. `/act` on an act session (e.g. the second leg
                    // of a `/plan` -> `/act` round trip) must stay silent.
                    if session.agent.name == a.name {
                        return Ok(());
                    }
                    // Replace the WHOLE agent struct and refresh the pool
                    // snapshots (tools PATH dirs + skill roots) so subsequent
                    // turns rebuild the system prompt from the new agent and
                    // the skill choke points / bash PATH follow it.
                    session.agent = a;
                    crate::agent_pools::refresh(session);
                    persist_agent(session, name).await?;
                    on_event(SessionEvent::AgentSwitch(name.clone()));
                }
                // Unknown/unresolvable name: name it in an Error event instead
                // of silently no-opping — a typo'd `/agent <name>` must tell
                // the user the switch did not happen. Still `Ok(())` so the
                // drain loop consumes the input rather than retrying forever.
                // (`/act`//`/plan` can never reach this arm: their names are
                // builtins and always resolve.)
                None => on_event(SessionEvent::Error(format!(
                    "unknown agent `{name}` — switch not applied"
                ))),
            }
        }
        ControlCmd::AgentList => {
            on_event(SessionEvent::Status(agent_listing(&session.agent.name)));
        }
        ControlCmd::ClearContext => {
            let plan_to_act = session.agent.kind == AgentKind::Plan;

            // A plan clear is an execution handoff, not a neutral history
            // fold: retain the newest real plan under HANDOFF_PREFIX so the
            // next act turn has an explicit instruction to implement it.
            // Other modes keep the existing neutral last-say seed contract.
            let directive_ready =
                plan_to_act && crate::handoff::reset_to_directive(session, "").is_some();
            if !directive_ready {
                fold_to_continuity_seed(session);
            }

            // Clear BOTH skill locks (body + names) via the shared seam: a
            // body-only clear left `active_skill_names` stale, keeping latent
            // tools unlocked across the clear boundary.
            crate::skill_lifecycle::clear_skill_state(session);
            let switched = plan_to_act.then(|| {
                let agent = resolve_agent("act").expect("built-in act agent must exist");
                let name = agent.name.clone();
                session.agent = agent;
                // Converged to a builtin: drop any file-agent pool surfaces
                // the plan session may have carried.
                crate::agent_pools::refresh(session);
                name
            });
            // Persist the boundary and converged agent atomically so resume
            // cannot resurrect plan mode behind an act handoff.
            persist_clear(session).await?;
            on_event(SessionEvent::TranscriptReset(session.messages.clone()));
            if let Some(name) = switched {
                on_event(SessionEvent::AgentSwitch(name));
            }
        }
    }
    Ok(())
}

/// Fold a non-plan transcript to one neutral continuity seed. This is also
/// the plan fallback when no real assistant plan exists.
fn fold_to_continuity_seed(session: &mut SessionState) {
    let store_msg_count = session.store_message_count();
    let preserved_images = crate::compaction::collect_head_images(&session.messages);
    let (mut marker, boundary) = match crate::handoff::last_assistant_text(&session.messages) {
        Some(last_say) => {
            let last_say = last_say.trim().to_string();
            (seed_message(&last_say), clear_seed_marker(&last_say))
        }
        None => {
            // No assistant text in the live transcript. The common cause is a
            // RE-clear (second Shift+Tab confirm, resume-then-clear, clear
            // before any act output): the transcript here holds ONLY
            // synthetic messages — the previous clear's boundary marker —
            // which `last_assistant_text` can never see again. If that
            // previous clear preserved a boundary (a directive display or a
            // continuity seed in `handoff_plan`), re-fold it AS-IS instead of
            // overwriting it with the blank sentinel: the sentinel would
            // silently drop the preserved plan both from the UI (the Plan
            // card rebuild filters it) and from the model. The marker
            // rebuild mirrors resume.rs so the in-memory transcript and a
            // later resume reconstruct the exact same flavour.
            match session.handoff_plan.clone() {
                Some(prev) if !prev.is_empty() && !is_clear_context_handoff(&prev) => {
                    if is_clear_context_seed(&prev) {
                        (seed_message(clear_seed_text(&prev)), prev)
                    } else {
                        (crate::handoff::handoff_message(&prev), prev)
                    }
                }
                // Genuinely nothing preserved anywhere: blank fresh start.
                _ => (fresh_start_message(), CLEAR_CONTEXT_SENTINEL.to_string()),
            }
        }
    };
    for url in &preserved_images {
        marker.blocks.push(ContentBlock::Image {
            url: url.clone(),
            detail: None,
        });
    }
    session.messages = vec![marker];
    session.after_handoff(store_msg_count as i64, boundary);
}

/// Build the synthetic fresh-start marker message. Exposed so [`crate::resume`]
/// can reconstruct the exact same message on resume.
pub fn fresh_start_message() -> Message {
    let mut msg = Message::user(new_id(), CLEAR_CONTEXT_BODY);
    msg.synthetic = true;
    msg
}

/// Build the synthetic seed message carrying the preserved last assistant
/// reply into the fresh transcript. Exposed so [`crate::resume`] can
/// reconstruct the exact same message after a seed boundary.
pub fn seed_message(text: &str) -> Message {
    let body = format!("{CLEAR_SEED_BODY_PREFIX}{text}");
    let mut msg = Message::user(new_id(), body);
    msg.synthetic = true;
    msg
}

/// One-line agent listing for the bare `/agent` command: primary builtin
/// agents plus every file-based agent card (builtins win on a name
/// collision), the session's current agent marked with a leading `*`.
/// Pure: filesystem access is the agents-root listing, which degrades to
/// empty when no root exists.
pub fn agent_listing(current: &str) -> String {
    let mut names: Vec<String> = builtin_agents()
        .into_iter()
        .filter(|a| a.mode == AgentMode::Primary)
        .map(|a| a.name)
        .collect();
    for name in list_agents() {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    let joined = names
        .iter()
        .map(|n| {
            if n == current {
                format!("*{n}")
            } else {
                n.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("agents: {joined}")
}

/// Persist an agent switch to the store. Closes the latent
/// resume-persistence gap where a switch via the TUI key handler was not
/// durably recorded: the worker now calls this so `resume()` and the `/task`
/// picker read the switched agent.
pub async fn persist_agent(session: &SessionState, agent: &str) -> Result<()> {
    if let Some(store) = &session.store {
        store
            .update_session(
                &session.id,
                &SessionPatch {
                    agent: Some(agent.into()),
                    updated_at: Some(now_ms()),
                    ..Default::default()
                },
            )
            .await?;
    }
    Ok(())
}

/// Persist the clear-context boundary: handoff metadata + active agent.
async fn persist_clear(session: &SessionState) -> Result<()> {
    if let Some(store) = &session.store {
        store
            .update_session(
                &session.id,
                &SessionPatch {
                    agent: Some(session.agent.name.clone()),
                    handoff_seq: session.handoff_seq,
                    handoff_plan: session.handoff_plan.clone(),
                    clear_summary: true,
                    clear_skill: true,
                    updated_at: Some(now_ms()),
                    ..Default::default()
                },
            )
            .await?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "control_cmd_tests.rs"]
mod tests;
