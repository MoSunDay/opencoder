//! TUI helpers for control-command (`/plan`, `/act`, `/act_clear_context`)
//! input handling. Functions are consumed by the idle/steer/queue submit
//! paths in `app.rs`.

#[cfg(test)]
#[path = "control_helpers_tests/mod.rs"]
mod tests;

/// When skill-token stripping collapses a compound control command (e.g.
/// "/plan $review" -> "/plan") to a bare command, forward the original text so
/// the runner's compound-command path resolves the skill and injects the
/// trigger. Otherwise return the cleaned text unchanged.
pub(crate) fn forward_skill_if_compound(raw: &str, clean: &str) -> String {
    if opencoder_session::parse_control_cmd(clean).is_some() && raw.trim() != clean {
        raw.trim().to_string()
    } else {
        clean.to_string()
    }
}

/// When the input is a **compound** `/plan <content>` submission (i.e. `/plan`
/// followed by substantive text or a `$skill` token), return the trimmed input
/// so the caller submits it as a plan-mode prompt instead of merely toggling
/// the agent. Bare `/plan`, whitespace-padded `/plan   `, `/act <content>`, and
/// plain text all return `None` (fall back to a normal mode toggle).
///
/// `split_control_prefix` is the single source of truth for "is there real
/// trailing content?": its internal `split_whitespace` strips every kind of
/// inter-token whitespace, so `/plan   ` (only trailing spaces) resolves to a
/// bare command (`rest == None`) and is correctly treated as a toggle.
pub(crate) fn plan_compound_for_submit(input: &str) -> Option<String> {
    match opencoder_session::split_control_prefix(input) {
        Some((opencoder_session::ControlCmd::SwitchAgent(mode), Some(_rest))) if mode == "plan" => {
            Some(input.trim().to_string())
        }
        _ => None,
    }
}

/// True when `clean` is a **compound** `/plan <content>` prompt (real trailing
/// content or a `$skill` token), as opposed to a bare mode toggle. A compound
/// `/plan` delivered from act mode arms the *deferred* plan->act handoff: the
/// submit/steer/queue paths set `ChatView::pending_plan_arm`, which the
/// `AgentSwitch("plan")` event consumes to re-arm `plan_submitted`.
pub(crate) fn is_compound_plan_cmd(clean: &str) -> bool {
    plan_compound_for_submit(clean).is_some()
}
