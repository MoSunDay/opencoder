//! Inline `$name` skill-token resolution for compound control commands.
//!
//! When a user submits a compound prompt like `/sandbox $review the api`, the
//! leading `/sandbox` switches the agent (handled by [`crate::control_cmd`]) and
//! the trailing text may carry `$skill` tokens. This module strips those
//! tokens, discovers the named skills, and activates their bodies on the
//! session — mirroring the TUI's `$`-picker path but operating directly on the
//! session state so headless / queue / steer submissions get the same
//! treatment.
//!
//! Latent tools (ssh_pty) unlock automatically: the runner
//! derives unlocked tools from the skill *body* text
//! ([`crate::tools::latent::unlocked_from_body`]), so simply setting the body
//! is sufficient — no separate tool-registry update is needed here.

use std::collections::HashSet;

use opencoder_core::message::now_ms;
use opencoder_core::{body_with_source, discover_skills, extract_skill_tokens, Message, Skill};
use opencoder_store::SessionPatch;

use crate::runner::new_id;
use crate::SessionState;

/// Synthetic trigger injected when `$skill` token stripping empties the text
/// but a skill was activated (e.g. `/sandbox $review` or a pure `$review` queue
/// item). Mirrors the idle path's pure-skill trigger so the model begins
/// executing the active skill (surfaced via the `[active skill]` tail
/// reminder). Read-only tagging is deliberately NOT applied to
/// this trigger, matching the idle path. One-shot: the skill this trigger
/// announces is cleared at the end of the run that consumed the token
/// (`skill_lifecycle`), so it never announces a stale skill on a later run.
pub const SKILL_TRIGGER: &str = "The active skill is now in effect. Begin executing it now.";

/// Persist the session's active skill to the store when it differs from
/// `prev` (the body captured *before* a resolution step). Mirrors the TUI's
/// `skill_persist::persist_skill`: best-effort — store errors are swallowed
/// because the in-memory write keeps the in-flight turn correct — and a
/// `None -> Some` transition only writes `skill`, never `clear_skill` (skill
/// *clearing* is owned by the explicit clear paths — control_cmd, autopilot
/// handoff, the TUI `$` menu — plus the run-end auto-clear in
/// `skill_lifecycle::clear_on_run_end`; this function never clears).
///
/// This is what makes consumption-time activation survive resume: the queue /
/// steer drain resolves `$name` tokens at the idle boundary and this call
/// lands the body in `sessions.skill` right then, so a crash mid-run and the
/// subsequent resume replays the drained item's post-state — including the
/// still-active skill — until that resumed run completes and the run-end
/// clear lands.
pub async fn persist_active_skill(session: &SessionState, prev: &Option<String>) {
    let Some(store) = session.store.clone() else {
        return;
    };
    let cur = session.skill_prompt_cloned();
    if cur.as_deref() == prev.as_deref() {
        return;
    }
    let _ = store
        .update_session(
            &session.id,
            &SessionPatch {
                skill: cur,
                updated_at: Some(now_ms()),
                ..Default::default()
            },
        )
        .await;
}

/// Resolve `$name` skill tokens in `text` against an *explicit* skill slice,
/// activating resolved skills on the session. Returns the text with tokens
/// stripped together with the names that matched no discovered skill (so
/// callers can warn).
///
/// When `text` has no tokens the active skill is left untouched for the
/// remainder of the current run (this function is pure activation — the
/// runner clears the skill at run end via `skill_lifecycle`). When tokens
/// are present, the resolved bodies are joined (`\\n\\n`) and set
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
                bodies.push(body_with_source(sk));
                resolved_names.insert(name.clone());
            }
            None => unresolved.push(name.clone()),
        }
    }
    if !bodies.is_empty() {
        session.set_skill(Some(bodies.join("\n\n")));
        session.set_active_skill_names(resolved_names.clone());
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
    let clean = resolve_inline_skills_with(session, text, &discover_skills()).0;
    // Expand `@path` mentions to absolute paths before the message is
    // recorded (direct-prompt path; steer/queue get the same treatment
    // via the head hook in `record_compound`).
    crate::mention_resolve::expand_mentions(&clean, &session.working_dir)
}

/// Record a prompt as a synthetic user message after resolving inline
/// `$skill` tokens. Used by the queue-drain and steer paths for both compound
/// commands (`/sandbox review`)
/// and plain prompts (`$review do it`) so both get consistent skill handling.
///
/// When THIS input resolved at least one `$skill` token and the stripping
/// empties the text (e.g. `/sandbox $review`), injects [`SKILL_TRIGGER`]
/// instead — mirroring the idle path's pure-skill behavior — and skips the
/// read-only tag. The condition is scoped to tokens resolved by this very
/// call, NOT the session's already-active skill: a queue/steer restart with
/// a stale active skill must not re-inject a trigger for a skill the item
/// never mentioned (that amplified the drain self-continuation loop).
/// Activations made here are one-shot: the run consuming this input clears
/// them at its end (`skill_lifecycle`).
pub async fn record_compound(session: &mut SessionState, rest: &str, images: &[String]) {
    // Expand `@path` mentions to absolute paths first so the recorded user
    // message (and the model request) carry full paths — the steer/queue
    // twin of the tail hook in `resolve_inline_skills`.
    let rest = &crate::mention_resolve::expand_mentions(rest, &session.working_dir);
    let skills = discover_skills();
    let prev_skill = session.skill_prompt_cloned();
    let (text, unresolved) = resolve_inline_skills_with(session, rest, &skills);
    persist_active_skill(session, &prev_skill).await;
    // "Resolved now": THIS input carried at least one `$name` token that
    // extraction found and discovery matched (so it was stripped and
    // activated above). Scoped to this call — not the already-active skill —
    // so a queue/steer restart with a stale active skill cannot re-inject a
    // trigger for a skill the item never mentioned.
    let resolved_now = extract_skill_tokens(rest)
        .1
        .iter()
        .any(|name| skills.iter().any(|s| &s.name == name) && !unresolved.contains(name));
    // Pure-skill: tokens consumed all text AND at least one resolved here.
    // Inject the trigger so the model acts on the skill body.
    if text.trim().is_empty() && images.is_empty() {
        if resolved_now {
            let mut msg = Message::user(new_id(), SKILL_TRIGGER);
            msg.synthetic = true;
            session.record(msg).await;
        }
        return;
    }
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
            source: PathBuf::from(format!("/skills/{name}/SKILL.md")),
        }
    }

    #[test]
    fn no_tokens_returns_text_unchanged_and_leaves_skill() {
        let s = make_session();
        let (clean, unresolved) = resolve_inline_skills_with(&s, "review the code", &[]);
        assert_eq!(clean, "review the code");
        assert!(unresolved.is_empty());
        assert!(
            s.skill_prompt_cloned().is_none(),
            "no-token call leaves skill untouched"
        );
    }

    #[test]
    fn resolves_single_skill_and_strips_token() {
        let s = make_session();
        let skills = vec![skill("review", "REVIEW BODY")];
        let (clean, unresolved) = resolve_inline_skills_with(&s, "$review do it", &skills);
        assert_eq!(clean, " do it");
        assert!(unresolved.is_empty());
        assert_eq!(
            s.skill_prompt_cloned().as_deref(),
            Some("> Source: /skills/review/SKILL.md\n\nREVIEW BODY")
        );
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
            "no resolved skill -> untouched"
        );
    }

    #[test]
    fn multiple_skills_joined() {
        let s = make_session();
        let skills = vec![skill("review", "R"), skill("submit", "S")];
        let (clean, unresolved) = resolve_inline_skills_with(&s, "$review $submit go", &skills);
        assert_eq!(clean, "  go");
        assert!(unresolved.is_empty());
        assert_eq!(
            s.skill_prompt_cloned().as_deref(),
            Some(
                "> Source: /skills/review/SKILL.md\n\nR\n\n> Source: /skills/submit/SKILL.md\n\nS"
            )
        );
    }

    #[test]
    fn mixed_resolved_and_unresolved() {
        let s = make_session();
        let skills = vec![skill("review", "R")];
        let (clean, unresolved) = resolve_inline_skills_with(&s, "$review $bogus", &skills);
        // Resolved `review` stripped; unresolved `$bogus` preserved verbatim.
        assert_eq!(clean, " $bogus");
        assert_eq!(unresolved, vec!["bogus"]);
        assert_eq!(
            s.skill_prompt_cloned().as_deref(),
            Some("> Source: /skills/review/SKILL.md\n\nR")
        );
    }

    #[test]
    fn dedupes_repeated_skill_name() {
        let s = make_session();
        let skills = vec![skill("review", "R")];
        let (_, unresolved) = resolve_inline_skills_with(&s, "$review $review", &skills);
        assert!(unresolved.is_empty());
        assert_eq!(
            s.skill_prompt_cloned().as_deref(),
            Some("> Source: /skills/review/SKILL.md\n\nR")
        );
    }

    #[tokio::test]
    async fn record_compound_records_cleaned_text() {
        let mut s = make_session();
        s.agent = resolve_agent("sandbox").unwrap();
        record_compound(&mut s, "review the code", &[]).await;
        assert_eq!(s.messages.len(), 1);
        assert!(
            !s.messages[0].synthetic,
            "text path records as real user input"
        );
        assert!(
            s.messages[0].text().contains("review the code"),
            "cleaned text recorded"
        );
    }

    #[tokio::test]
    async fn record_compound_pure_skill_injects_trigger() {
        let mut s = make_session();
        s.agent = resolve_agent("sandbox").unwrap();
        {
            let _guard = lock_home(tempfile::tempdir().unwrap().path());
            opencoder_core::seed_builtin_skills();
            // A pure `$review` token resolves against the seeded skill and
            // empties the text -> trigger injected.
            record_compound(&mut s, "$review", &[]).await;
        }
        assert_eq!(s.messages.len(), 1);
        assert_eq!(s.messages[0].text(), SKILL_TRIGGER, "trigger injected");
        assert!(s.messages[0].synthetic);
        assert!(
            s.skill_prompt_cloned().is_some(),
            "skill activated by the token"
        );
    }

    #[tokio::test]
    async fn record_compound_persists_resolved_skill_to_store() {
        let home = tempfile::tempdir().unwrap();
        let _guard = lock_home(home.path());
        opencoder_core::seed_builtin_skills();
        let store: std::sync::Arc<dyn opencoder_store::Store> =
            std::sync::Arc::new(opencoder_store::LibsqlStore::open_memory().await.unwrap());
        store
            .create_session(&opencoder_store::SessionMeta {
                id: "sess-skill".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let mut s = make_session().with_store(store.clone());

        record_compound(&mut s, "$review do it", &[]).await;

        let persisted = store
            .get_session("sess-skill")
            .await
            .unwrap()
            .and_then(|m| m.skill);
        assert_eq!(
            persisted,
            s.skill_prompt_cloned(),
            "consumption-time activation lands in sessions.skill verbatim"
        );
        assert!(
            persisted
                .as_deref()
                .is_some_and(|b| b.starts_with("> Source: ")),
            "persisted body carries the source prefix: {persisted:?}"
        );
    }

    #[tokio::test]
    async fn record_compound_active_skill_without_token_records_nothing() {
        let mut s = make_session();
        // An already-active skill (e.g. resumed mid-run) must NOT be
        // re-triggered by an empty queue/steer item that carries no `$token`
        // of its own.
        s.set_skill(Some("STALE BODY".into()));
        record_compound(&mut s, "", &[]).await;
        assert!(
            s.messages.is_empty(),
            "no trigger for empty text with only an already-active skill"
        );
    }

    #[tokio::test]
    async fn record_compound_unresolved_token_does_not_inject() {
        let mut s = make_session();
        s.set_skill(Some("STALE BODY".into()));
        {
            let _guard = lock_home(tempfile::tempdir().unwrap().path());
            // `$bogus` matches no discovered skill: the token survives as
            // literal text, so a REAL user message is recorded — but no
            // SKILL_TRIGGER (nothing resolved now; the already-active skill
            // alone is not a trigger source).
            record_compound(&mut s, "$bogus", &[]).await;
        }
        assert_eq!(s.messages.len(), 1, "literal unresolved token recorded");
        assert_eq!(s.messages[0].text(), "$bogus");
        assert!(!s.messages[0].synthetic, "no synthetic trigger injected");
    }

    #[tokio::test]
    async fn record_compound_empty_no_skill_records_nothing() {
        let mut s = make_session();
        record_compound(&mut s, "", &[]).await;
        assert!(s.messages.is_empty(), "nothing recorded for empty no-skill");
    }

    // ---- HOME isolation for discover_skills (mirrors tests/drain_mode.rs) ----

    static HOME_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct HomeGuard {
        prev_home: Option<std::ffi::OsString>,
        prev_xdg: Option<std::ffi::OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    fn lock_home(home: &std::path::Path) -> HomeGuard {
        let _lock = HOME_MUTEX.lock().unwrap();
        let prev_home = std::env::var_os("HOME");
        let prev_xdg = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("HOME", home);
        std::env::set_var("XDG_CONFIG_HOME", home);
        HomeGuard {
            prev_home,
            prev_xdg,
            _lock,
        }
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            match self.prev_home.take() {
                Some(h) => std::env::set_var("HOME", h),
                None => std::env::remove_var("HOME"),
            }
            match self.prev_xdg.take() {
                Some(h) => std::env::set_var("XDG_CONFIG_HOME", h),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
        }
    }
}
