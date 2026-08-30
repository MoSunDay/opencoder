//! Latent tool gating: tools that exist in the registry but are hidden from
//! the model until a corresponding skill is activated via `$skill-name`.
//!
//! This is the third filtering layer (after `ToolFilter` and the registry
//! itself). A latent tool passes the agent allowlist but is still withheld
//! unless its owning skill's name is in the session's `active_skill_names` set.
//!
//! There are no agent-kind exemptions: `question` is task-plan-unlocked
//! uniformly for every agent (sandbox included), and nothing outside the
//! task-plan skill body advertises it.

use std::collections::HashSet;

/// All tool names that are latent (hidden until their skill is activated).
const LATENT_TOOLS: &[&str] = &["question", "ssh_pty"];

/// True when `name` is a latent tool.
pub fn is_latent_tool(name: &str) -> bool {
    LATENT_TOOLS.contains(&name)
}

/// Returns the tool names unlocked by activating `skill_name`.
/// Returns an empty slice for unknown / non-latent skills.
pub fn latent_tools_for_skill(skill_name: &str) -> &'static [&'static str] {
    match skill_name {
        "task-plan" => &["question"],
        "ssh-pty" => &["ssh_pty"],
        _ => &[],
    }
}

/// Skill names whose body text unlocks the `question` tool.
const QUESTION_SKILLS: &[&str] = &["task-plan"];

/// The full visibility rule for a registry tool under the latent-gating
/// layer: agent allowlist ∧ latent unlock. `question` is task-plan-unlocked
/// uniformly for every agent (sandbox included) — no agent-kind exemption —
/// and nothing advertises it outside the task-plan skill body. `ssh_pty` is
/// skill-gated everywhere. Shared by the runner's tool filter and the token
/// estimator so the advertised schema array and its cost estimate never drift.
pub fn is_visible(name: &str, agent: &opencoder_core::Agent, unlocked: &HashSet<&str>) -> bool {
    agent.tools.allows(name) && (!is_latent_tool(name) || unlocked.contains(name))
}

/// Compute the set of latent tool names that should be unlocked, given the
/// currently active skill names. Non-latent tools are never included.
pub fn unlocked_tools(skill_names: &HashSet<String>) -> HashSet<&'static str> {
    let mut out = HashSet::new();
    for name in skill_names {
        for tool in latent_tools_for_skill(name) {
            out.insert(*tool);
        }
    }
    out
}

/// Derive unlocked latent tools from a skill prompt body. Used by the runner
/// to unlock tools without a separate `active_skill_names` registry — the body
/// text already contains skill-specific identifiers that we match against.
///
/// Only the first 500 chars are inspected: the seeded `SKILL.md` files name
/// their skill and the `question` tool up front (see the seed contract tests
/// in `opencoder-core`), so a deep scan would leak unlocks to bodies that
/// merely reference a skill in passing.
pub fn unlocked_from_body(body: Option<&str>) -> HashSet<&'static str> {
    let mut out = HashSet::new();
    if let Some(b) = body {
        let prefix: String = b.chars().take(500).collect();
        if prefix.contains("ssh_pty") || prefix.contains("ssh-pty") {
            for t in latent_tools_for_skill("ssh-pty") {
                out.insert(*t);
            }
        }
        if QUESTION_SKILLS.iter().any(|s| prefix.contains(s)) {
            for t in latent_tools_for_skill("task-plan") {
                out.insert(*t);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_pty_is_latent() {
        assert!(is_latent_tool("ssh_pty"));
    }

    #[test]
    fn question_is_latent() {
        assert!(is_latent_tool("question"));
    }

    #[test]
    fn normal_tools_not_latent() {
        assert!(!is_latent_tool("bash"));
        assert!(!is_latent_tool("read"));
        assert!(!is_latent_tool("task"));
    }

    #[test]
    fn skill_to_tool_mapping() {
        assert_eq!(latent_tools_for_skill("ssh-pty"), &["ssh_pty"]);
        assert_eq!(latent_tools_for_skill("task-plan"), &["question"]);
        assert!(
            latent_tools_for_skill("review").is_empty(),
            "review must not unlock any latent tool (question is task-plan-only)"
        );
        assert!(latent_tools_for_skill("unknown").is_empty());
    }

    #[test]
    fn unlocked_tools_from_skill_names() {
        let mut names = HashSet::new();
        names.insert("ssh-pty".to_string());
        names.insert("task-plan".to_string());
        let unlocked = unlocked_tools(&names);
        assert!(unlocked.contains("ssh_pty"));
        assert!(unlocked.contains("question"));
    }

    #[test]
    fn unlocked_from_body_detects_ssh_pty() {
        let body = Some("# ssh-pty skill\n\nYou have ssh_pty...");
        let unlocked = unlocked_from_body(body);
        assert!(unlocked.contains("ssh_pty"));
    }

    #[test]
    fn unlocked_from_body_detects_plan_skill_only() {
        // The seeded body mentions its own skill name in the first 500 chars
        // (contract-tested in opencoder-core); that unlocks question.
        let plan_body = Some("# task-plan\n\n## Overview\n\nMulti-phase planning...");
        assert!(unlocked_from_body(plan_body).contains("question"));
    }

    #[test]
    fn unlocked_from_body_review_skill_unlocks_nothing() {
        // review no longer owns the question tool: its body must never
        // unlock it, even when the body names itself up front.
        let review_body = Some("# review\n\nEvidence-driven assessment of shipped work.");
        assert!(unlocked_from_body(review_body).is_empty());
    }

    #[test]
    fn body_mention_beyond_prefix_unlocks_nothing() {
        // A body that only references the skill name past the 500-char window
        // (e.g. a random skill that cross-links task-plan) must NOT unlock.
        let filler = "x".repeat(600);
        let late = format!("{filler} task-plan question");
        assert!(unlocked_from_body(Some(&late)).is_empty());
    }

    #[test]
    fn question_tool_name_alone_does_not_unlock() {
        // Matching the tool name is deliberately not enough — only the
        // task-plan skill name unlocks question.
        let body = Some("ask via the question tool whenever blocked");
        assert!(!unlocked_from_body(body).contains("question"));
    }

    /// Skills root whose absolute path is at least `min_len` bytes deep
    /// (nested components under a tempdir), mimicking a real-world deep HOME.
    /// Returns the tempdir (kept alive by the caller) and the skills root.
    fn deep_skills_root(min_len: usize) -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let mut root = tmp.path().to_path_buf();
        let mut level = 0;
        while root.as_os_str().len() < min_len {
            root.push(format!("lvl{level:02}"));
            level += 1;
        }
        std::fs::create_dir_all(&root).unwrap();
        (tmp, root)
    }

    #[test]
    fn long_source_path_keeps_question_within_unlock_window() {
        // Production bodies carry a `> Source: <HOME>/skills/task-plan/SKILL.md`
        // line (`opencoder_core::body_with_source`). A real deep HOME pushes
        // the seeded body's `question` mention out of the 500-char window —
        // the unlock must STILL fire, because the skill name rides inside the
        // Source line itself. Built from the real seed asset landed on a deep
        // path via `seed_builtin_skills_in` (session depends on core), so the
        // test dies if the seed content or the annotation format drifts.
        let (_home, root) = deep_skills_root(240);
        opencoder_core::seed_builtin_skills_in(&root).unwrap();
        let plan = opencoder_core::discover_in(&root)
            .into_iter()
            .find(|s| s.name == "task-plan")
            .expect("seeded task-plan skill");
        let injected = opencoder_core::body_with_source(&plan);
        let prefix: String = injected.chars().take(500).collect();
        assert!(
            prefix.starts_with("> Source: "),
            "fixture must mirror the production body_with_source shape: {prefix}"
        );
        assert!(
            plan.source.as_os_str().len() >= 240,
            "fixture: source path must be a deep HOME (>= 240 bytes), got {}",
            plan.source.as_os_str().len()
        );
        assert!(
            !prefix.contains("question"),
            "fixture: the long Source line must push the question mention past \
             the 500-char window (this is the exact risk under test)"
        );
        assert!(
            unlocked_from_body(Some(&injected)).contains("question"),
            "a deep-HOME Source line must not silently drop the question unlock"
        );
    }

    #[test]
    fn long_source_path_review_body_still_unlocks_nothing() {
        // Control: the same long Source line in front of a body that carries
        // no task-plan protocol must not unlock. The seeded review body even
        // mentions `question` verbatim (its clarification protocol forbids
        // calling it) — the unlock keys on the task-plan skill name only, so
        // a deep HOME must not turn that mention into a unlock either.
        let (_home, root) = deep_skills_root(240);
        opencoder_core::seed_builtin_skills_in(&root).unwrap();
        let review = opencoder_core::discover_in(&root)
            .into_iter()
            .find(|s| s.name == "review")
            .expect("seeded review skill");
        let injected = opencoder_core::body_with_source(&review);
        assert!(
            unlocked_from_body(Some(&injected)).is_empty(),
            "review body with a deep Source line must unlock no latent tool"
        );
    }

    #[test]
    fn act_without_skill_unlocks_nothing() {
        // An act session with no skill body: no latent unlocks at all.
        assert!(unlocked_from_body(None).is_empty());
        assert!(unlocked_from_body(Some("plain execution prompt")).is_empty());
    }

    #[test]
    fn unlocked_from_body_none_when_no_skill() {
        assert!(unlocked_from_body(None).is_empty());
        assert!(unlocked_from_body(Some("random text")).is_empty());
    }

    #[test]
    fn unknown_skill_unlocks_nothing() {
        let mut names = HashSet::new();
        names.insert("bogus".to_string());
        assert!(unlocked_tools(&names).is_empty());
    }

    #[test]
    fn visibility_sandbox_needs_skill_unlock_too() {
        // No agent exemptions: the sandbox allowlist still carries `question`,
        // but the tool stays hidden until the task-plan skill unlocks it.
        let sandbox = opencoder_core::resolve_agent("sandbox").unwrap();
        let none = HashSet::new();
        assert!(
            sandbox.tools.allows("question"),
            "sandbox still allowlists question (unlock is runtime latent-gating)"
        );
        assert!(
            !is_visible("question", &sandbox, &none),
            "sandbox sees question only with the task-plan unlock"
        );
        let unlocked: HashSet<&str> = ["question"].into_iter().collect();
        assert!(is_visible("question", &sandbox, &unlocked));
        // Still bound by the agent allowlist: command/workflow agents do not.
        let command = opencoder_core::resolve_agent("command").unwrap();
        assert!(!is_visible("question", &command, &unlocked));
    }

    #[test]
    fn visibility_act_needs_skill_unlock() {
        let act = opencoder_core::resolve_agent("act").unwrap();
        let none = HashSet::new();
        assert!(
            !is_visible("question", &act, &none),
            "act without the task-plan skill must not see question"
        );
        let unlocked: HashSet<&str> = ["question"].into_iter().collect();
        assert!(is_visible("question", &act, &unlocked));
    }

    #[test]
    fn visibility_ssh_pty_unchanged() {
        // No builtin agent allowlists ssh_pty (it targets custom agents), so
        // build one: ssh_pty must stay purely skill-gated — no latent tool
        // has any agent exemption.
        let agent = opencoder_core::Agent {
            name: "ssh-host".into(),
            kind: opencoder_core::AgentKind::Act,
            mode: opencoder_core::AgentMode::Primary,
            description: String::new(),
            prompt: String::new(),
            tools: opencoder_core::ToolFilter::Allow(vec!["ssh_pty".into()]),
        };
        let none = HashSet::new();
        assert!(!is_visible("ssh_pty", &agent, &none));
        let unlocked: HashSet<&str> = ["ssh_pty"].into_iter().collect();
        assert!(is_visible("ssh_pty", &agent, &unlocked));
    }

    #[test]
    fn visibility_allowlist_still_applies() {
        let explore = opencoder_core::resolve_agent("explore").unwrap();
        let unlocked: HashSet<&str> = ["question", "ssh_pty"].into_iter().collect();
        // Unlocked-but-not-allowlisted stays hidden.
        assert!(!is_visible("question", &explore, &unlocked));
    }
}
