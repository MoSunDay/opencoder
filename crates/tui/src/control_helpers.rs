//! TUI helpers for control-command (`/plan`, `/act`, `/act_clear_context`)
//! input handling. Both functions are consumed by the idle/steer/queue submit
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

/// True when `clean` is a **bare** control command (`/plan`, `/act`,
/// `/act_clear_context`) with no trailing argument. Pure control commands
/// switch mode without recording a user message, so their text is NOT echoed
/// into the transcript. Compound inputs (`/plan $review`, `/plan fix it`)
/// carry user content and MUST be echoed before execution.
pub(crate) fn is_pure_control_cmd(clean: &str) -> bool {
    matches!(
        opencoder_session::split_control_prefix(clean),
        Some((_, None))
    )
}
