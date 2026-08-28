//! Latent tool gating: tools that exist in the registry but are hidden from
//! the model until a corresponding skill is activated via `$skill-name`.
//!
//! This is the third filtering layer (after `ToolFilter` and the registry
//! itself). A latent tool passes the agent allowlist but is still withheld
//! unless its owning skill's name is in the session's `active_skill_names` set.
//!
//! The sandbox agent is exempt for `question`: its clarification protocol is
//! part of the agent's base prompt, so the tool is always visible there (see
//! the visibility predicates in `runner::llm_call` / `tools::estimate_tool_schema_tokens`).

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
        "task-plan" | "review" => &["question"],
        "ssh-pty" => &["ssh_pty"],
        _ => &[],
    }
}

/// Skill names whose body text unlocks the `question` tool.
const QUESTION_SKILLS: &[&str] = &["task-plan", "review"];

/// The full visibility rule for a registry tool under the latent-gating
/// layer: agent allowlist ∧ latent unlock — with one sandbox exemption.
/// The sandbox agent's clarification protocol lives in its base prompt, so
/// `question` is ALWAYS visible there (bypasses latent gating, matching the
/// pre-refactor plan-mode behavior). Every other agent (act, subagents) must
/// unlock `question` through the task-plan / review skill; `ssh_pty` is
/// skill-gated everywhere. Shared by the runner's tool filter and the token
/// estimator so the advertised schema array and its cost estimate never drift.
pub fn is_visible(name: &str, agent: &opencoder_core::Agent, unlocked: &HashSet<&str>) -> bool {
    if name == "question" && agent.kind == opencoder_core::AgentKind::Sandbox {
        return true;
    }
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
        assert_eq!(latent_tools_for_skill("review"), &["question"]);
        assert!(latent_tools_for_skill("unknown").is_empty());
    }

    #[test]
    fn unlocked_tools_from_skill_names() {
        let mut names = HashSet::new();
        names.insert("ssh-pty".to_string());
        names.insert("review".to_string());
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
    fn unlocked_from_body_detects_plan_and_review_skills() {
        // The seeded bodies mention their own skill name in the first 500
        // chars (contract-tested in opencoder-core); either unlocks question.
        let plan_body = Some("# task-plan\n\n## Overview\n\nMulti-phase planning...");
        assert!(unlocked_from_body(plan_body).contains("question"));

        let review_body = Some("# review\n\nEvidence-driven assessment of shipped work.");
        assert!(unlocked_from_body(review_body).contains("question"));
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
        // task-plan / review skill names unlock question.
        let body = Some("ask via the question tool whenever blocked");
        assert!(!unlocked_from_body(body).contains("question"));
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
    fn visibility_sandbox_always_sees_question() {
        let sandbox = opencoder_core::resolve_agent("sandbox").unwrap();
        let none = HashSet::new();
        assert!(
            is_visible("question", &sandbox, &none),
            "sandbox sees question with no skill at all"
        );
        // Still bound by the agent allowlist: command/workflow agents do not.
        let command = opencoder_core::resolve_agent("command").unwrap();
        assert!(!is_visible("question", &command, &none));
    }

    #[test]
    fn visibility_act_needs_skill_unlock() {
        let act = opencoder_core::resolve_agent("act").unwrap();
        let none = HashSet::new();
        assert!(
            !is_visible("question", &act, &none),
            "act without the task-plan/review skill must not see question"
        );
        let unlocked: HashSet<&str> = ["question"].into_iter().collect();
        assert!(is_visible("question", &act, &unlocked));
    }

    #[test]
    fn visibility_ssh_pty_unchanged() {
        // No builtin agent allowlists ssh_pty (it targets custom agents), so
        // build one: ssh_pty must stay purely skill-gated — the sandbox
        // question exemption must not leak to it.
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
