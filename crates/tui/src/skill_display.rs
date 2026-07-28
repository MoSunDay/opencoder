//! Pure helpers for formatting pure-skill (`{$name}`) submissions.
//!
//! Extracted from `app_helpers` to keep that module within the line budget.
//! The trigger text is what the store admits (the LLM needs the full
//! instruction); the display token is what the queue/steer panels show the
//! user so they see the original `{$name}` they submitted.

/// Build the synthetic prompt sent when a user submits ONLY a skill token
/// (`{$name}` with no accompanying text) — i.e. a pure-skill submission. The
/// skill body itself is injected into the system prompt; this trigger text just
/// records a user turn and tells the model to begin acting on the skill. Used by
/// the Submit (idle), Steer (running), and Queue (running) paths so a pure-skill
/// submission is never silently dropped regardless of the submit verb.
pub(crate) fn skill_trigger(skill_name: &str) -> String {
    format!("The `{skill_name}` skill is now active. Begin executing its instructions immediately.")
}

/// Display string for a pure-skill submission in the queue/steer panels and
/// transcript markers. The full trigger (see [`skill_trigger`]) is still
/// admitted to the store for the LLM; this returns the original `{$name}`
/// token so the user sees what they actually submitted rather than the
/// synthetic trigger description.
pub(crate) fn skill_token_display(skill_name: &str) -> String {
    format!("{{${skill_name}}}")
}
