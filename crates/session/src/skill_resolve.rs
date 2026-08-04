//! Inline `$name` skill-token resolution for compound control commands.
//!
//! When a user submits a compound prompt like `/plan $review the api`, the
//! leading `/plan` switches the agent (handled by [`crate::control_cmd`]) and
//! the trailing text may carry `$skill` tokens. This module strips those
//! tokens, discovers the named skills, and activates their bodies on the
//! session — mirroring the TUI's `$`-picker path but operating directly on the
//! session state so headless / queue / steer submissions get the same
//! treatment.
//!
//! Latent tools (ssh_pty, chrome_headless) unlock automatically: the runner
//! derives unlocked tools from the skill *body* text
//! ([`crate::tools::latent::unlocked_from_body`]), so simply setting the body
//! is sufficient — no separate tool-registry update is needed here.

use std::collections::HashSet;

use opencoder_core::{discover_skills, extract_skill_tokens, Message, Skill};

use crate::runner::new_id;
use crate::SessionState;

/// Synthetic trigger injected when `$skill` token stripping empties the text
/// but a skill was activated (e.g. `/plan $review` or a pure `$review` queue
/// item). Mirrors the idle path's pure-skill trigger so the model begins
/// executing the skill body already injected into the system prompt. The
/// plan-mode read-only tag is deliberately NOT applied to this trigger,
/// matching the idle path.
pub const SKILL_TRIGGER: &str = "The active skill is now in effect. Begin executing it now.";

/// Resolve `$name` skill tokens in `text` against an *explicit* skill slice,
/// activating resolved skills on the session. Returns the text with tokens
/// stripped together with the names that matched no discovered skill (so
/// callers can warn).
///
/// When `text` has no tokens the active skill is left untouched (sticky).
/// When tokens are present, the resolved bodies are joined (`\\n\\n`) and set
/// as the session skill, matching the TUI multi-skill convention. Only
/// resolved `$name` tokens are stripped from the returned text; unresolved
/// `$name` sequences are preserved verbatim as literal text.
pub fn resolve_inline_skills_with(
    session: &SessionState,
    text: &str,
    skills: &[Skill],
) -> (String, Vec<String>) {
    let (clean, names) = extract_skill_tokens(text);
    if names.is_empty() {
        return (clean, Vec::new());
    }
    // Dedupe preserving first-seen order (mirrors the TUI resolver).
    let mut seen: HashSet<&str> = HashSet::new();
    let mut bodies: Vec<String> = Vec::new();
    let mut resolved_names: HashSet<String> = HashSet::new();
    let mut unresolved: Vec<String> = Vec::new();
    for name in &names {
        if !seen.insert(name.as_str()) {
            continue;
        }
        match skills.iter().find(|s| &s.name == name) {
            Some(sk) => {
                bodies.push(sk.body.clone());
                resolved_names.insert(name.clone());
            }
            None => unresolved.push(name.clone()),
        }
    }
    if !bodies.is_empty() {
        session.set_skill(Some(bodies.join("\n\n")));
    }
    // Rebuild `clean` so ONLY resolved tokens are stripped — unresolved `$name`
    // bytes stay as literal text, preventing content loss.
    let clean = opencoder_core::strip_resolved_skill_tokens(text, &resolved_names);
    (clean, unresolved)
}

/// Discover skills from `~/.opencoder/skills` and resolve inline `$name`
/// tokens in `text`, activating resolved skills on the session. Returns the
/// cleaned text (tokens stripped); unresolved names are silently ignored.
pub fn resolve_inline_skills(session: &SessionState, text: &str) -> String {
    resolve_inline_skills_with(session, text, &discover_skills()).0
}

/// Record a prompt as a synthetic user message after resolving inline
/// `$skill` tokens and applying the plan-mode read-only tag. Used by the
/// queue-drain and steer paths for both compound commands (`/plan review`)
/// and plain prompts (`$review do it`) so both get consistent skill handling.
///
/// When `$skill` stripping empties the text but a skill was activated (e.g.
/// `/plan $review`), injects [`SKILL_TRIGGER`] instead — mirroring the idle
/// path's pure-skill behavior — and skips the plan-mode tag.
pub async fn record_compound(session: &mut SessionState, rest: &str, images: &[String]) {
    let mut text = resolve_inline_skills(session, rest);
    // Pure-skill: tokens consumed all text but activated a skill. Inject the
    // trigger so the model acts on the skill body (no plan-mode tag, matching
    // the idle path).
    if text.trim().is_empty() && images.is_empty() {
        if session.skill_prompt_cloned().is_some() {
            let mut msg = Message::user(new_id(), SKILL_TRIGGER);
            msg.synthetic = true;
            session.record(msg).await;
        }
        return;
    }
    session.maybe_tag_plan_prompt(&mut text);
    let m = Message::user_with_images(new_id(), text, images);
    session.record(m).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    use opencoder_core::{resolve_agent, Config};
    use opencoder_llm::{ChatStream, MockChatClient};

    fn make_session() -> SessionState {
        let working_dir = std::env::temp_dir().join("opencoder-skill-resolve-tests");
        SessionState::new(
            "sess-skill",
            resolve_agent("act").unwrap(),
            Config::default(),
            Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
            working_dir,
        )
    }

    fn skill(name: &str, body: &str) -> Skill {
        Skill {
            name: name.into(),
            description: String::new(),
            body: body.into(),
            source: PathBuf::new(),
        }
    }

    #[test]
    fn no_tokens_returns_text_unchanged_and_leaves_skill() {
        let s = make_session();
        let (clean, unresolved) = resolve_inline_skills_with(&s, "review the code", &[]);
        assert_eq!(clean, "review the code");
        assert!(unresolved.is_empty());
        assert!(s.skill_prompt_cloned().is_none(), "sticky: skill untouched");
    }

    #[test]
    fn resolves_single_skill_and_strips_token() {
        let s = make_session();
        let skills = vec![skill("review", "REVIEW BODY")];
        let (clean, unresolved) = resolve_inline_skills_with(&s, "$review do it", &skills);
        assert_eq!(clean, " do it");
        assert!(unresolved.is_empty());
        assert_eq!(s.skill_prompt_cloned().as_deref(), Some("REVIEW BODY"));
    }

    #[test]
    fn unresolved_skill_reported_and_skill_untouched() {
        let s = make_session();
        let (clean, unresolved) = resolve_inline_skills_with(&s, "$bogus text", &[]);
        // Unresolved `$bogus` is preserved verbatim (no content loss).
        assert_eq!(clean, "$bogus text");
        assert_eq!(unresolved, vec!["bogus"]);
        assert!(
            s.skill_prompt_cloned().is_none(),
            "no resolved skill -> sticky"
        );
    }

    #[test]
    fn multiple_skills_joined() {
        let s = make_session();
        let skills = vec![skill("review", "R"), skill("submit", "S")];
        let (clean, unresolved) = resolve_inline_skills_with(&s, "$review $submit go", &skills);
        assert_eq!(clean, "  go");
        assert!(unresolved.is_empty());
        assert_eq!(s.skill_prompt_cloned().as_deref(), Some("R\n\nS"));
    }

    #[test]
    fn mixed_resolved_and_unresolved() {
        let s = make_session();
        let skills = vec![skill("review", "R")];
        let (clean, unresolved) = resolve_inline_skills_with(&s, "$review $bogus", &skills);
        // Resolved `review` stripped; unresolved `$bogus` preserved verbatim.
        assert_eq!(clean, " $bogus");
        assert_eq!(unresolved, vec!["bogus"]);
        assert_eq!(s.skill_prompt_cloned().as_deref(), Some("R"));
    }

    #[test]
    fn dedupes_repeated_skill_name() {
        let s = make_session();
        let skills = vec![skill("review", "R")];
        let (_, unresolved) = resolve_inline_skills_with(&s, "$review $review", &skills);
        assert!(unresolved.is_empty());
        assert_eq!(s.skill_prompt_cloned().as_deref(), Some("R"));
    }

    #[tokio::test]
    async fn record_compound_records_cleaned_text() {
        let mut s = make_session();
        s.agent = resolve_agent("plan").unwrap();
        // First plan input (count 0) -> no read-only tag appended.
        record_compound(&mut s, "review the code", &[]).await;
        assert_eq!(s.messages.len(), 1);
        assert!(!s.messages[0].synthetic, "text path records as real user input");
        assert!(
            s.messages[0].text().contains("review the code"),
            "cleaned text recorded"
        );
        assert_eq!(s.plan_input_count, 1, "plan input counter incremented");
    }

    #[tokio::test]
    async fn record_compound_pure_skill_injects_trigger() {
        let mut s = make_session();
        s.agent = resolve_agent("plan").unwrap();
        // Seed the skill first, then call record_compound with only the token.
        s.set_skill(Some("REVIEW BODY".into()));
        record_compound(&mut s, "$review", &[]).await;
        assert_eq!(s.messages.len(), 1);
        assert_eq!(s.messages[0].text(), SKILL_TRIGGER, "trigger injected");
        assert!(s.messages[0].synthetic);
        // plan_input_count NOT incremented (trigger skips the plan tag).
        assert_eq!(s.plan_input_count, 0, "trigger skips plan-mode counter");
    }

    #[tokio::test]
    async fn record_compound_empty_no_skill_records_nothing() {
        let mut s = make_session();
        record_compound(&mut s, "", &[]).await;
        assert!(s.messages.is_empty(), "nothing recorded for empty no-skill");
    }
}
