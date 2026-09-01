//! Latent tool gating: tools that exist in the registry but are hidden from
//! the model until a corresponding skill is activated via `$skill-name`.
//!
//! This is the third filtering layer (after `ToolFilter` and the registry
//! itself). A latent tool passes the agent allowlist but is still withheld
//! unless its owning skill's name is in the session's `active_skill_names` set.
//!
//! The plan agent is exempt for `question`: its clarification protocol is
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
        "task-plan" => &["question"],
        "ssh-pty" => &["ssh_pty"],
        _ => &[],
    }
}

/// Skill names whose body text unlocks the `question` tool.
const QUESTION_SKILLS: &[&str] = &["task-plan"];

/// The full visibility rule for a registry tool under the latent-gating
/// layer: agent allowlist ∧ latent unlock — with one plan exemption.
/// The plan agent's clarification protocol lives in its base prompt, so
/// `question` is ALWAYS visible there (bypasses latent gating, matching the
/// pre-refactor plan-mode behavior). Every other agent (act, subagents) must
/// unlock `question` through the task-plan skill; `ssh_pty` is
/// skill-gated everywhere. Shared by the runner's tool filter and the token
/// estimator so the advertised schema array and its cost estimate never drift.
pub fn is_visible(name: &str, agent: &opencoder_core::Agent, unlocked: &HashSet<&str>) -> bool {
    if name == "question" && agent.kind == opencoder_core::AgentKind::Plan {
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

/// Prefix `opencoder_core::body_with_source` puts in front of every injected
/// skill body (and each section of a compound activation).
const SOURCE_LINE_PREFIX: &str = "> Source: ";

/// Extract the skill name from a `> Source: <path>` annotation line. Directory
/// layout (`.../skills/<name>/SKILL.md`) resolves to the directory name under
/// `skills/`; flat layout (`.../skills/<name>.md`) to the file stem. Matching
/// is EXACT on the resolved name, so a lookalike user skill (`my-task-plan`)
/// never unlocks a builtin's tools. Returns `None` for non-Source lines and
/// paths without a recognizable `skills/` segment.
fn skill_name_from_source_line(line: &str) -> Option<&str> {
    let path = line.strip_prefix(SOURCE_LINE_PREFIX)?.trim();
    let mut segments = path.split('/').filter(|s| !s.is_empty());
    while let Some(seg) = segments.next() {
        if seg != "skills" {
            continue;
        }
        let name = segments.next()?;
        let stem = name.strip_suffix(".md").unwrap_or(name);
        return if stem.is_empty() { None } else { Some(stem) };
    }
    None
}

/// Derive unlocked latent tools from a skill prompt body. Used by the runner
/// to unlock tools without a separate `active_skill_names` registry - the body
/// text already contains skill-specific identifiers that we match against.
///
/// Primary path: every `> Source: <path>` annotation line (the
/// `body_with_source` shape) is resolved to its skill name and matched
/// exactly against the latent-skill table. The decision is
/// position-independent, so a compound activation (several joined sections)
/// unlocks even when a section lands past the first 500 chars, and the old
/// substring match can no longer mis-unlock a `my-task-plan` user skill.
///
/// Legacy fallback: a body with NO Source line (direct `set_skill` callers,
/// tests, pre-annotation sessions) keeps the historical behaviour - scan only
/// the first 500 chars for the skill identifiers, so a passing mention deeper
/// in a body still does not leak unlocks.
/// Canonicalise a resolved skill name to the static set of skill names that
/// carry builtin latent semantics. Unknown (user) skill names return `None`
/// and never enter the active-name set.
fn known_skill_name(name: &str) -> Option<&'static str> {
    match name {
        "task-plan" => Some("task-plan"),
        "ssh-pty" => Some("ssh-pty"),
        _ => None,
    }
}

/// Resolve the set of active builtin skill names from a skill prompt body.
/// Primary path: every `> Source: <path>` line resolves EXACTLY (a lookalike
/// `my-task-plan` never matches). Legacy fallback: a body with NO Source line
/// (direct `set_skill` callers, tests, pre-annotation sessions) scans only the
/// first 500 chars for the skill identifiers. Shared by latent-tool unlock
/// ([`unlocked_from_body`]) and the task-plan prompt/schema stripping
/// ([`task_plan_active`]) so every consumer agrees on activation.
pub fn active_skill_names(body: Option<&str>) -> HashSet<&'static str> {
    let mut out = HashSet::new();
    let Some(b) = body else {
        return out;
    };
    let mut saw_source = false;
    for line in b.lines() {
        let Some(name) = skill_name_from_source_line(line) else {
            continue;
        };
        saw_source = true;
        if let Some(name) = known_skill_name(name) {
            out.insert(name);
        }
    }
    if saw_source {
        return out;
    }
    let prefix: String = b.chars().take(500).collect();
    if prefix.contains("ssh_pty") || prefix.contains("ssh-pty") {
        out.insert("ssh-pty");
    }
    if QUESTION_SKILLS.iter().any(|s| prefix.contains(s)) {
        out.insert("task-plan");
    }
    out
}

/// Derive the unlocked latent tool names from a skill prompt body — the
/// runner's advertisement/exec-time unlock source. A thin projection of
/// [`active_skill_names`] through the skill-to-tool table.
pub fn unlocked_from_body(body: Option<&str>) -> HashSet<&'static str> {
    active_skill_names(body)
        .into_iter()
        .flat_map(|name| latent_tools_for_skill(name).iter().copied())
        .collect()
}

/// True when the task-plan skill is active. Same EXACT resolution as the
/// latent layer, so the prompt/schema build-stripping lights up precisely
/// when the skill that owns the plan-only contract is committed (the same
/// condition that turns the TUI `act` chip yellow).
pub fn task_plan_active(body: Option<&str>) -> bool {
    active_skill_names(body).contains("task-plan")
}

/// Execution-time re-check for the generic tool dispatcher: a latent tool may
/// only execute while its owning skill is still active. Defence in depth
/// against hallucinated calls - the schema array already withholds latent
/// tools, but the registry keeps them callable, and `question` has no
/// wall-clock budget (`leaf_tool_timeout`), so an unguarded phantom ask could
/// park the run forever. Same unlock contract as advertisement-time gating
/// ([`unlocked_from_body`]); non-latent tools are always allowed.
pub fn latent_execution_allowed(name: &str, skill_body: Option<&str>) -> bool {
    !is_latent_tool(name) || unlocked_from_body(skill_body).contains(name)
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
    fn task_plan_active_via_source_line() {
        let body = Some("> Source: /home/u/.opencoder/skills/task-plan/SKILL.md\n\nbody");
        assert!(task_plan_active(body));
        // Flat layout resolves to the file stem.
        let flat = Some("> Source: /skills/task-plan.md\n\nbody");
        assert!(task_plan_active(flat));
    }

    #[test]
    fn task_plan_active_rejects_lookalike_and_other_skills() {
        let lookalike = Some("> Source: /skills/my-task-plan/SKILL.md\n\nbody");
        assert!(!task_plan_active(lookalike));
        let review = Some("> Source: /skills/review/SKILL.md\n\nbody");
        assert!(!task_plan_active(review));
        assert!(!task_plan_active(None));
    }

    #[test]
    fn task_plan_active_legacy_prefix_fallback() {
        // Bodies without a Source line keep the 500-char prefix scan.
        assert!(task_plan_active(Some("# task-plan\nplan the work")));
        let filler = "x".repeat(600);
        let late = format!("{filler} task-plan");
        assert!(!task_plan_active(Some(&late)));
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

    #[test]
    fn execution_gate_refuses_calls_the_body_does_not_unlock() {
        // The pure predicate behind the dispatcher's re-check: non-latent
        // tools always pass; latent tools pass only when the body unlocks.
        assert!(latent_execution_allowed("bash", None));
        assert!(latent_execution_allowed("read", Some("# task-plan")));
        assert!(!latent_execution_allowed("question", None));
        assert!(!latent_execution_allowed("ssh_pty", None));
        assert!(latent_execution_allowed(
            "question",
            Some("# task-plan\n\nask via question when blocked")
        ));
        assert!(latent_execution_allowed(
            "ssh_pty",
            Some("> Source: /h/skills/ssh-pty/SKILL.md\n\nattach with ssh_pty")
        ));
        assert!(!latent_execution_allowed(
            "question",
            Some("> Source: /h/skills/my-task-plan/SKILL.md\n\nplan")
        ));
    }

    #[test]
    fn compound_source_sections_unlock_in_both_orders() {
        // Compound activation joins `> Source: <path>\n\n<body>` sections
        // (`skill_resolve::resolve_inline_skills_with`). A section that lands
        // after a long first body sits past the legacy 500-char window, so the
        // unlock must come from the Source lines themselves - in BOTH orders.
        let filler = format!("phase one detail. {}\n\n", "x".repeat(600));
        let plan = "> Source: /home/u/.opencoder/skills/task-plan/SKILL.md\n\n\
                    plan the launch; ask via question when blocked.";
        let ssh = "> Source: /home/u/.opencoder/skills/ssh-pty/SKILL.md\n\n\
                   attach terminals with ssh_pty.";
        let plan_first = format!("{filler}{plan}\n\n{ssh}");
        let ssh_first = format!("{filler}{ssh}\n\n{plan}");
        for (label, body) in [("plan-first", &plan_first), ("ssh-first", &ssh_first)] {
            let unlocked = unlocked_from_body(Some(body));
            assert!(
                unlocked.contains("question"),
                "{label} must still unlock question: {unlocked:?}"
            );
            assert!(
                unlocked.contains("ssh_pty"),
                "{label} must still unlock ssh_pty: {unlocked:?}"
            );
        }
    }

    #[test]
    fn lookalike_user_skill_source_line_unlocks_nothing() {
        // Exact-name matching: `my-task-plan` is a DIFFERENT skill; its Source
        // line must not unlock the builtin task-plan's tools. (The old
        // `prefix.contains("task-plan")` substring match mis-unlocked here.)
        let body = "> Source: /home/u/skills/my-task-plan/SKILL.md\n\n\
                    plan carefully; ask via question when blocked.";
        assert!(unlocked_from_body(Some(body)).is_empty());
    }

    #[test]
    fn flat_source_file_stem_unlocks_ssh_pty() {
        // Flat skill layout: the file stem under `skills/` is the skill name.
        let body = "> Source: /etc/opencoder/skills/ssh-pty.md\n\nopen terminals.";
        assert!(unlocked_from_body(Some(body)).contains("ssh_pty"));
    }

    #[test]
    fn legacy_body_without_source_lines_still_scans_prefix() {
        // Fallback preserved: a body with NO `> Source:` annotation keeps the
        // historical 500-char prefix behaviour (and its window).
        assert!(
            unlocked_from_body(Some("# task-plan\n\nask via question")).contains("question"),
            "legacy bare task-plan body must keep unlocking question"
        );
        assert!(
            unlocked_from_body(Some("# ssh-pty\n\nuse ssh_pty")).contains("ssh_pty"),
            "legacy bare ssh-pty body must keep unlocking ssh_pty"
        );
        let filler = "x".repeat(600);
        let late = format!("{filler} task-plan question");
        assert!(
            unlocked_from_body(Some(&late)).is_empty(),
            "the legacy 500-char window still applies without a Source line"
        );
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
        // test dies if the seed content or the annotation format drifts. The
        // root is deep enough that the Source line ALONE fills the 500-char
        // window, so the contract holds no matter where the (agent-owned)
        // seed body first says `question`.
        let (_home, root) = deep_skills_root(520);
        opencoder_core::seed_builtin_skills_in(&root.join("skills")).unwrap();
        let plan = opencoder_core::discover_in(&root.join("skills"))
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
        opencoder_core::seed_builtin_skills_in(&root.join("skills")).unwrap();
        let review = opencoder_core::discover_in(&root.join("skills"))
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
    fn visibility_plan_always_sees_question() {
        let plan = opencoder_core::resolve_agent("plan").unwrap();
        let none = HashSet::new();
        assert!(
            is_visible("question", &plan, &none),
            "plan sees question with no skill at all"
        );
        // Other kinds still need the task-plan unlock.
        let act = opencoder_core::resolve_agent("act").unwrap();
        assert!(
            !is_visible("question", &act, &none),
            "act sees question only with the task-plan unlock"
        );
        let unlocked: HashSet<&str> = ["question"].into_iter().collect();
        assert!(is_visible("question", &act, &unlocked));
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
        // build one: ssh_pty must stay purely skill-gated — the plan
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
