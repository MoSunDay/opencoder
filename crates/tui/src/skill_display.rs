//! Pure helpers around pure-skill (`$name`) submissions and persisted skill
//! bodies.
//!
//! The trigger text is what the idle Submit sends (the LLM needs the full
//! instruction); queued/steered submissions defer entirely — the raw text
//! (token included) is admitted and the runner's `record_compound` resolves
//! it at the idle boundary.

/// Build the synthetic prompt sent when a user submits ONLY a skill token
/// (`$name` with no accompanying text) while idle — i.e. a pure-skill
/// submission. The skill itself is surfaced via the context-tail reminder;
/// this trigger text just records a user turn and tells the model to begin
/// acting on the skill. (The running paths no longer build triggers here:
/// a queued/steered `$name` is admitted verbatim and `record_compound`
/// injects its own `SKILL_TRIGGER` at consumption.)
pub(crate) fn skill_trigger(skill_name: &str) -> String {
    format!("The `{skill_name}` skill is now active. Begin executing its instructions immediately.")
}

/// Derive a display skill name from a persisted body's `> Source:` prefix
/// (`.../skills/<name>/SKILL.md` -> `<name>`). Used to re-sync the TUI's
/// local `active_skill` mirror after the runner activated a skill at
/// consumption time (queue/steer drain): the runner shares only the body
/// through the `skill_prompt` Arc, never the name. For multi-skill joined
/// bodies the first block's name wins (display only — the full body still
/// drives the tail reminder and latent-tool unlocks).
pub(crate) fn skill_name_from_body(body: &str) -> Option<String> {
    let path = opencoder_session::skill_context::source_path_from_body(body)?;
    std::path::Path::new(path)
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_trigger_names_the_active_skill() {
        assert!(skill_trigger("repo-memory").contains("`repo-memory`"));
        assert!(skill_trigger("x").contains("`x`"));
    }

    #[test]
    fn name_derived_from_source_prefix() {
        let body = "> Source: /skills/haiku/SKILL.md\n\nAlways answer in haiku form.";
        assert_eq!(skill_name_from_body(body).as_deref(), Some("haiku"));
    }

    #[test]
    fn multi_skill_body_uses_first_block() {
        let body =
            "> Source: /skills/review/SKILL.md\n\nR\n\n> Source: /skills/submit/SKILL.md\n\nS";
        assert_eq!(skill_name_from_body(body).as_deref(), Some("review"));
    }

    #[test]
    fn body_without_source_prefix_has_no_name() {
        assert_eq!(skill_name_from_body("just instructions"), None);
        assert_eq!(skill_name_from_body(""), None);
    }
}
