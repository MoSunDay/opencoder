//! Pure helpers for formatting pure-skill (`$name`) submissions.
//!
//! Extracted from `app_helpers` to keep that module within the line budget.
//! The trigger text is what the store admits (the LLM needs the full
//! instruction); the display token is what the queue/steer panels show the
//! user so they see the original `$name` they submitted.

/// Build the synthetic prompt sent when a user submits ONLY a skill token
/// (`$name` with no accompanying text) — i.e. a pure-skill submission. The
/// skill body itself is injected into the system prompt; this trigger text just
/// records a user turn and tells the model to begin acting on the skill. Used by
/// the Submit (idle), Steer (running), and Queue (running) paths so a pure-skill
/// submission is never silently dropped regardless of the submit verb.
pub(crate) fn skill_trigger(skill_name: &str) -> String {
    format!("The `{skill_name}` skill is now active. Begin executing its instructions immediately.")
}

/// Display string for a pure-skill submission in the queue/steer panels and
/// transcript markers. The full trigger (see [`skill_trigger`]) is still
/// admitted to the store for the LLM; this returns the original `$name`
/// token so the user sees what they actually submitted rather than the
/// synthetic trigger description.
pub(crate) fn skill_token_display(skill_name: &str) -> String {
    format!("${skill_name}")
}

/// Display string for a queued/steered combined submission (`$skill text`) in
/// the side panels and the `queued:`/`steer:` consumed markers.
///
/// The store row admits only the token-stripped `clean` text (the LLM and the
/// web drain must never see the token), so the queue panel — the only place a
/// queued item is surfaced, it is not echoed in the transcript — would show
/// just `text`, making the inserted `$skill` silently vanish. Mirroring the
/// Submit transcript (which records the raw input), the UI shows exactly what
/// the user typed whenever the token stripping changed anything; plain text
/// has no tokens, so `text` and `clean` coincide and this is a pass-through.
pub(crate) fn queued_item_display(text: &str, clean: &str) -> String {
    let raw = text.trim();
    if raw == clean {
        clean.to_string()
    } else {
        raw.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_passes_through_clean() {
        assert_eq!(queued_item_display("fix the bug", "fix the bug"), "fix the bug");
    }

    #[test]
    fn combined_skill_keeps_token_visible() {
        assert_eq!(
            queued_item_display("$repo-memory fix the bug", "fix the bug"),
            "$repo-memory fix the bug"
        );
    }

    #[test]
    fn whitespace_only_difference_uses_clean() {
        assert_eq!(queued_item_display("  fix  ", "fix"), "fix");
    }

    #[test]
    fn mid_text_token_preserved() {
        assert_eq!(
            queued_item_display("do $a then $b", "do  then "),
            "do $a then $b"
        );
    }
}
