//! TUI helpers for control-command (`/act`, `/plan`, `/act_clear_context`)
//! input handling. Functions are consumed by the idle/steer/queue submit
//! paths in `app.rs`.

#[cfg(test)]
#[path = "control_helpers_tests/mod.rs"]
mod tests;

/// When skill-token stripping collapses a compound control command (e.g.
/// "/plan $review" -> "/plan") to a bare command, forward the original
/// text so the runner's compound-command path resolves the skill and injects
/// the trigger. Otherwise return the cleaned text unchanged.
pub(crate) fn forward_skill_if_compound(raw: &str, clean: &str) -> String {
    if opencoder_session::parse_control_cmd(clean).is_some() && raw.trim() != clean {
        raw.trim().to_string()
    } else {
        clean.to_string()
    }
}
