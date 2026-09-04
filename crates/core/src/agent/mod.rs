//! Agent registry: builtin agents plus resolution of file-based custom
//! agents from the shared resource pools under `~/.opencoder/agents/`
//! ([`meta`], [`resource`], [`compose`]). A custom agent is a thin
//! reference card (`<name>/meta.json`) naming shared prompt/skills/tools/
//! memory pool resources — two agents referencing the same prompt share
//! it. Builtin names always win — a custom agent can never shadow `act`
//! or `plan`; file agents resolve read-only and degrade to `None` on any
//! filesystem/parse failure, so callers fall back to builtin behavior.

use serde::{Deserialize, Serialize};

use crate::config::Config;

pub mod compose;
pub mod meta;
pub mod resource;

pub use compose::compose_prompt;
pub use meta::{
    active_agent, agent_dir, agents_dir, list_agents, read_agent_meta, set_active_agent,
    set_active_agent_checked, set_agents_dir_override, validate_agent_name, AgentHistoryEntry,
    AgentMeta, AgentRefs, AgentReferences,
};
pub use resource::{
    active_skill_roots, active_tools_dirs, agent_skill_roots, agent_tools_dirs, all_tools_dirs,
    category_dir, list_resources, read_resource_meta, resource_current_version_dir,
    resource_version_dir, validate_resource_name, AGENT_CATEGORIES, ResourceMeta,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentMode {
    Primary,
    Subagent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    Act,
    /// The read-only planning mode. During the sandbox-mode interlude this
    /// kind was serialized as `"sandbox"`; the alias keeps old persisted
    /// payloads (session state, events) deserializing after the revert.
    #[serde(alias = "sandbox")]
    Plan,
    Subagent,
    Command,
    Workflow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolFilter {
    All,
    Allow(Vec<String>),
}

impl ToolFilter {
    pub fn allows(&self, name: &str) -> bool {
        match self {
            ToolFilter::All => true,
            ToolFilter::Allow(list) => list.iter().any(|t| t == name),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Agent {
    pub name: String,
    pub kind: AgentKind,
    pub mode: AgentMode,
    pub description: String,
    pub prompt: String,
    pub tools: ToolFilter,
}

impl Agent {
    pub fn is_primary(&self) -> bool {
        self.mode == AgentMode::Primary
    }
}

/// Resolve an agent by name. Builtin names win unconditionally (a file
/// agent can never shadow `act`/`plan`/subagents); on a builtin miss the
/// file-based agent under `agents/<name>/` is resolved read-only — any
/// failure (invalid name, missing/corrupt meta, unreadable prompt version)
/// degrades to `None` so callers fall back to builtin behavior.
pub fn resolve_agent(name: &str) -> Option<Agent> {
    if let Some(builtin) = builtin_agents().into_iter().find(|a| a.name == name) {
        return Some(builtin);
    }
    resolve_file_agent(name)
}

/// Resolve a file-based custom agent's reference card into an [`Agent`].
/// Path safety first: the name must pass [`meta::validate_agent_name`]
/// before any filesystem access. The card's `current.prompt` reference
/// must resolve to a shared `prompts/<ref>/` pool version with at least
/// one readable section (`soul.md`/`how.md`/`output.md`, each optional);
/// all three missing/blank ⇒ not a real agent ⇒ `None`. A `current.memory`
/// reference that resolves appends a `# Memory` section. Any failure
/// (stale refs, corrupt metas, unreadable files) degrades to `None`.
fn resolve_file_agent(name: &str) -> Option<Agent> {
    if meta::validate_agent_name(name).is_err() {
        return None;
    }
    let card = meta::read_agent_meta(name)?;
    // No prompt reference ⇒ not a resolvable agent.
    let prompt_ref = card.current.prompt?;
    let dir = meta::resource_current_version_dir("prompts", &prompt_ref)?;
    // Each section file is optional; a missing file is simply `None`.
    let read = |file: &str| std::fs::read_to_string(dir.join(format!("{file}.md"))).ok();
    let (soul, how, output) = (read("soul"), read("how"), read("output"));
    let mut prompt = compose_prompt(soul.as_deref(), how.as_deref(), output.as_deref());
    if prompt.is_empty() {
        return None; // all sections missing/blank — not a real agent
    }
    // Shared memory pool: a resolving ref appends a `# Memory` section.
    if let Some(memory_ref) = card.current.memory.as_deref() {
        if let Some(body) = meta::resource_current_version_dir("memory", memory_ref)
            .and_then(|d| std::fs::read_to_string(d.join("memory.md")).ok())
        {
            prompt.push_str("\n\n# Memory\n");
            prompt.push_str(body.trim());
        }
    }
    // Description: the first non-empty soul line (a one-line identity),
    // else a stable generic label.
    let description = soul
        .as_deref()
        .and_then(|s| s.lines().map(str::trim).find(|l| !l.is_empty()))
        .map(str::to_string)
        .unwrap_or_else(|| format!("Custom agent {name}"));
    Some(Agent {
        name: name.into(),
        kind: AgentKind::Act,
        mode: AgentMode::Primary,
        description,
        prompt,
        tools: ToolFilter::All,
    })
}

/// The effective default agent name for a fresh session:
/// `cli_override` > the active file agent ([`meta::active_agent`]) >
/// `cfg.agent.default` (when non-empty) > `"act"`. Blank strings at any
/// tier are skipped (an empty CLI flag or config value must not win over a
/// real resolution).
pub fn effective_default_agent(cli_override: Option<&str>, cfg: &Config) -> String {
    if let Some(o) = cli_override.map(str::trim).filter(|s| !s.is_empty()) {
        return o.to_string();
    }
    if let Some(active) = meta::active_agent() {
        return active;
    }
    let cfg_default = cfg.agent.default.trim();
    if !cfg_default.is_empty() {
        return cfg_default.to_string();
    }
    default_agent_name().to_string()
}

pub fn default_agent_name() -> &'static str {
    "act"
}

pub fn builtin_agents() -> Vec<Agent> {
    vec![
        Agent {
            name: "act".into(),
            kind: AgentKind::Act,
            mode: AgentMode::Primary,
            description: "Default execution agent. Orchestrates work via bash and subagents.".into(),
            prompt: base_prompt_act(),
            tools: ToolFilter::Allow(vec!["bash".into(), "task".into(), "question".into()]),
        },
        Agent {
            name: "plan".into(),
            kind: AgentKind::Plan,
            mode: AgentMode::Primary,
            description: "Read-only plan agent. Explores and answers questions; mutating operations are intercepted.".into(),
            prompt: base_prompt_plan(),
            tools: ToolFilter::Allow(vec![
                "bash".into(), "task".into(),
                // Latent tool: gated by the task-plan skill everywhere;
                // the plan-kind exemption lives in tools::latent::is_visible.
                "question".into(),
            ]),
        },
        Agent {
            name: "explore".into(),
            kind: AgentKind::Subagent,
            mode: AgentMode::Subagent,
            description: "Read-only subagent for exploring codebases: find files, search code, read files, answer questions. Cannot modify files.".into(),
            prompt: base_prompt_explore(),
            tools: ToolFilter::Allow(vec![
                "search".into(), "read".into(),
            ]),
        },
        Agent {
            name: "build".into(),
            kind: AgentKind::Subagent,
            mode: AgentMode::Subagent,
            description: "Implementation subagent: bash (terminal ops, reading files) and edit (precise code changes). Use for making code changes.".into(),
            prompt: base_prompt_build(),
            tools: ToolFilter::Allow(vec![
                "bash".into(), "edit".into(),
            ]),
        },
        Agent {
            name: "sidecar".into(),
            kind: AgentKind::Subagent,
            mode: AgentMode::Subagent,
            description: "Sidecar observer: a temporary bypass loop that answers questions about the main task's progress from a context snapshot. Read-only, makes no changes.".into(),
            prompt: base_prompt_sidecar(),
            tools: ToolFilter::Allow(vec![
                "read".into(), "search".into(), "ls".into(), "bash".into(),
            ]),
        },
        Agent {
            name: "command".into(),
            kind: AgentKind::Command,
            mode: AgentMode::Primary,
            description: "One-shot single-turn agent. Runs a single prompt to completion without interactive follow-up.".into(),
            prompt: base_prompt_act(),
            tools: ToolFilter::Allow(vec!["bash".into(), "task".into()]),
        },
        Agent {
            name: "workflow".into(),
            kind: AgentKind::Workflow,
            mode: AgentMode::Primary,
            description: "Internal durable scheduler for todos workflows.".into(),
            prompt: "You are the scheduler and acceptance brain for a durable TODO workflow. Return exactly one JSON object matching the operation schema in the user prompt. Never emit markdown or prose outside JSON. Use only the supplied state and references; never invent execution evidence.".into(),
            tools: ToolFilter::Allow(Vec::new()),
        },
    ]
}

pub fn base_prompt_act() -> String {
    BASE_PROMPT.to_string()
}

/// Bash + subagent usage preamble appended to a custom `--prompt-file` prompt.
///
/// It advertises the `bash` and `task` tools and the `explore`/`build`
/// delegation, so a user-supplied role prompt still drives correct tool use.
/// The `'build'` delegation clause matches the substring targeted by
/// `base_prompt_plan` for build-stripping in plan mode.
pub fn tool_preamble() -> &'static str {
    "## Tools
- You have two tools: bash (terminal ops: git, builds, tests, running scripts) and task (to spawn subagents).
- For file operations, delegate to subagents: use 'explore' (read-only) for investigation, 'build' (full tools) for implementation.
- Run tool calls in parallel when none needs the other's output; otherwise run sequentially. You MAY emit multiple `task` blocks in a single response -- independent subagents dispatched this way run concurrently, so prefer batching independent investigations.
- Keep responses concise and friendly. Do not dump large files; reference paths only.
- When a tool errors, read the error, fix the approach, and retry; do not loop on the same failing command.
"
}

/// The BASE_PROMPT / `tool_preamble` clause advertising the 'build' (full
/// tools) subagent. Stripping it yields a prompt that never tells the model
/// the build subagent exists: plan mode strips always (read-only), and an
/// act session strips while the task-plan skill is active (plan-only turns
/// must not be advertised implementation delegation).
pub const BUILD_DELEGATION_CLAUSE: &str = ", 'build' (full tools) for implementation";

/// Remove the 'build' subagent advertisement from a base-style prompt.
/// Shared by `base_prompt_plan` and the session layer's task-plan prompt
/// stripping, so the clause wording lives in exactly one place.
pub fn strip_build_delegation(prompt: &str) -> String {
    prompt.replace(BUILD_DELEGATION_CLAUSE, "")
}

/// Single source of truth for whether the 'build' subagent must be absent
/// from every model-facing surface (system prompt, tool schema, error
/// copy): plan mode always (read-only contract), plus any mode while the
/// task-plan skill is active (plan-only turns are not advertised
/// implementation delegation). Prompt stripping (`base_prompt_plan`, the
/// session's `build_system`, the CLI `--prompt-file` composer) and schema
/// hiding (`hide_build_subagent`) must all derive from this predicate so
/// the surfaces cannot drift.
pub fn build_delegation_hidden(kind: AgentKind, task_plan_active: bool) -> bool {
    kind == AgentKind::Plan || task_plan_active
}

pub fn base_prompt_plan() -> String {
    // Plan mode must not advertise the 'build' subagent: strip the build
    // delegation clause from the shared base prompt before appending the
    // plan suffix. Act mode keeps the full BASE_PROMPT unchanged.
    let base = strip_build_delegation(BASE_PROMPT);
    format!("{base}\n\n{}", PLAN_SUFFIX)
}

pub fn base_prompt_explore() -> String {
    "You are a read-only exploration subagent. Your job is to investigate the codebase and report findings. \
     You have search (ripgrep code search) and read tools. You CANNOT edit or write files. \
     Complete the specific task delegated to you, then return a concise summary of your findings. \
     Do not ask questions; infer reasonable defaults and proceed."
        .to_string()
}

pub fn base_prompt_build() -> String {
    "You are an implementation subagent. You have bash (terminal ops; use cat/grep/sed to read files) \
     and edit (precise string replacement) tools. Complete the specific task delegated to you: \
     inspect code, make edits, run bash commands, and verify your work. \
     Do not ask questions; infer reasonable defaults and proceed. \
     After finishing, briefly state what you changed and the key file paths."
        .to_string()
}

pub fn base_prompt_sidecar() -> String {
    "You are the sidecar observer of a main agent session: a temporary bypass loop that answers \
     questions about the main task's progress, status, or plan. The user message carries a \
     snapshot of the main session's conversation context as background - treat it as read-only \
     reference material. You have read, search, and ls tools, plus bash for read-only \
     inspection commands (git log, grep, wc) when the snapshot is not enough. Every \
     state-changing bash command is intercepted and refused - do not retry or look for \
     another write path. You CANNOT edit or write files and must never claim any change was made. \
     Answer concisely and progress-oriented: what is done, what is in flight, what comes next."
        .to_string()
}

const PLAN_SUFFIX: &str = "\
PLAN mode (read-only): no edits/writes and no implementation execution. Every state-changing tool attempt (including writes under /tmp) is intercepted and returned in context. If blocked, do not retry or look for another write path; focus on analysis and output a plan only. \
Investigate via 'explore' subagents.";

const BASE_PROMPT: &str = "\
You are OpenCoder, a high-performance coding agent in a terminal.

## How to work
- Default to doing the work without asking questions. Infer missing details by reading the codebase and following existing conventions.
- You have two tools: bash (for terminal ops: git, builds, tests, running scripts) and task (to spawn subagents).
- For file operations, delegate to subagents: use 'explore' (read-only) for investigation, 'build' (full tools) for implementation.
- Run tool calls in parallel when none needs the other's output; otherwise run sequentially.
- You MAY emit multiple `task` blocks in a single response. Independent subagents dispatched this way run concurrently, so prefer batching independent investigations.
- Keep responses concise and friendly. Do not dump large files; reference paths only.
- Only add comments when necessary.

## Editing
- Default to ASCII. Match existing file style.
- Never revert changes you did not make. Do not amend commits unless asked. Avoid destructive git commands (reset --hard, checkout --) unless explicitly requested.

## Tool results
- When a tool errors, read the error, fix the approach, and retry; do not loop on the same failing command.
- After finishing, briefly state what you did and the key files, and suggest logical next steps (tests, build, commit).
";

#[cfg(test)]
mod tests {
    use super::*;

    /// Guards the `.replace()` in `base_prompt_plan()`: if BASE_PROMPT's
    /// wording ever drifts so the replace becomes a no-op, the build subagent
    /// advertisement silently leaks into the plan prompt. These assertions
    /// fail loudly instead.
    #[test]
    fn plan_prompt_strips_build_subagent_advertisement() {
        // The exact clause targeted by `strip_build_delegation` (used by
        // `base_prompt_plan` and the session layer's task-plan stripping).
        // If this assertion fails, BASE_PROMPT has changed — update
        // `BUILD_DELEGATION_CLAUSE` to match the new wording.
        let replace_target = BUILD_DELEGATION_CLAUSE;
        assert!(
            base_prompt_act().contains(replace_target),
            "BASE_PROMPT no longer contains the '.replace()' target substring \
             {replace_target:?}. Update the .replace() call in base_prompt_plan()."
        );

        let plan = base_prompt_plan();

        // Safety property: the plan prompt must not advertise 'build'.
        assert!(
            !plan.contains("'build' (full tools)"),
            "plan prompt must not advertise the 'build' subagent, got: {plan}"
        );

        // Sanity: the 'explore' advertisement must survive (the replace should
        // only strip the build clause, not the entire delegation line).
        assert!(
            plan.contains("'explore' (read-only)"),
            "plan prompt must still advertise 'explore', got: {plan}"
        );
    }

    /// `question` is allowlisted for the two primary agents only (`plan` is
    /// exempt from latent gating, `act` needs the task-plan skill unlock;
    /// runtime visibility is gated elsewhere). Subagents never see it --
    /// zero schema token cost. Structural guard (rules/01) against filter
    /// drift.
    #[test]
    fn question_tool_is_plan_and_act_only() {
        for name in ["plan", "act"] {
            let a = resolve_agent(name).expect("primary agent registered");
            assert!(a.tools.allows("question"), "{name} must allow 'question'");
        }
        for other in ["explore", "build", "sidecar", "command", "workflow"] {
            let a = resolve_agent(other).expect("agent registered");
            assert!(
                !a.tools.allows("question"),
                "{other} must not allow 'question'"
            );
        }
    }

    /// Pin down the `sidecar` observer's tool set: read-only inspection only
    /// (read/search/ls plus a classifier-gated bash), never mutating or
    /// delegation tools. The sidecar answers questions about the main task
    /// from a context snapshot; it must never be able to change state.
    #[test]
    fn sidecar_observer_is_read_only() {
        let sidecar = resolve_agent("sidecar").expect("sidecar agent registered");
        assert_eq!(sidecar.kind, AgentKind::Subagent);
        assert_eq!(sidecar.mode, AgentMode::Subagent);
        for allowed in &["read", "search", "ls", "bash"] {
            assert!(
                sidecar.tools.allows(allowed),
                "sidecar must allow '{allowed}'"
            );
        }
        for blocked in &["edit", "write", "task", "question"] {
            assert!(
                !sidecar.tools.allows(blocked),
                "sidecar (read-only) must not allow '{blocked}'"
            );
        }
        // The prompt states the observer contract: snapshot-in, progress-out,
        // read-only bash, and no modification claims.
        let prompt = sidecar.prompt;
        assert!(prompt.contains("sidecar observer"), "got: {prompt}");
        assert!(
            prompt.contains("read-only inspection commands"),
            "got: {prompt}"
        );
        assert!(prompt.contains("intercepted and refused"), "got: {prompt}");
        assert!(prompt.contains("CANNOT edit or write"), "got: {prompt}");
    }

    /// The plan prompt requires a focused plan without reviving the old rigid
    /// Goal/TODO/Verify/Risks/Align template or an automatic act handoff.
    #[test]
    fn plan_prompt_is_read_only_without_question_advertisement() {
        let plan = base_prompt_plan();

        // Read-only constraints survive the rename.
        assert!(
            plan.contains("read-only"),
            "plan prompt must state its read-only constraints, got: {plan}"
        );
        assert!(
            plan.contains("Every state-changing tool attempt"),
            "plan prompt must note intercepted writes, got: {plan}"
        );
        assert!(plan.contains("output a plan only"), "got: {plan}");
        assert!(plan.contains("do not retry"), "got: {plan}");

        // No question-tool advertisement in the base prompt: the tool's
        // description lives ONLY in the task-plan skill body. (Generic prose
        // like "without asking questions" is fine — only the backticked
        // tool name or an explicit tool mention advertises the schema.)
        for banned in [
            "`question`",
            "prefer asking over assuming",
            "several in one turn",
            "looked up first, not asked",
        ] {
            assert!(
                !plan.contains(banned),
                "plan prompt must not advertise the question tool ({banned}), got: {plan}"
            );
        }

        // Plan-template semantics are gone.
        assert!(
            !plan.contains("Goal / TODO / Verify / Risks / Align"),
            "plan prompt must not require the plan template sections, got: {plan}"
        );
        assert!(
            !plan.contains("act mode"),
            "plan prompt must not hand off to a plan/act mode switch, got: {plan}"
        );
    }

    /// Pin down the `explore` subagent's exact tool set: it must carry
    /// **only** `search` + `read` — the read-only pair. This is a structural
    /// guard (rules/01): if the tool list drifts (e.g. an old `glob`/`grep`
    /// creeps back, or a mutating tool leaks in) the test fails loudly.
    #[test]
    fn explore_subagent_carries_search_and_read_only() {
        let explore = resolve_agent("explore").expect("explore subagent registered");
        assert_eq!(explore.mode, AgentMode::Subagent);
        // Positive: the two read-only tools must be present.
        assert!(
            explore.tools.allows("search"),
            "explore must allow 'search'"
        );
        assert!(explore.tools.allows("read"), "explore must allow 'read'");
        // Negative: no mutating, delegation, or removed tools may leak in.
        for blocked in &["bash", "edit", "task", "write", "glob", "grep", "ls"] {
            assert!(
                !explore.tools.allows(blocked),
                "explore (read-only) must not allow '{blocked}'"
            );
        }
    }

    /// Pin down the `build` subagent's exact tool set: it must carry
    /// **only** `bash` + `edit` — the implementation pair.
    #[test]
    fn build_subagent_carries_bash_and_edit_only() {
        let build = resolve_agent("build").expect("build subagent registered");
        assert_eq!(build.mode, AgentMode::Subagent);
        assert!(build.tools.allows("bash"), "build must allow 'bash'");
        assert!(build.tools.allows("edit"), "build must allow 'edit'");
        for blocked in &["search", "read", "task", "write", "glob", "grep", "ls"] {
            assert!(
                !build.tools.allows(blocked),
                "build (implementation) must not allow '{blocked}'"
            );
        }
    }

    /// The predicate every hide surface must derive from.
    #[test]
    fn build_delegation_hidden_matrix() {
        assert!(build_delegation_hidden(AgentKind::Plan, false));
        assert!(build_delegation_hidden(AgentKind::Plan, true));
        assert!(!build_delegation_hidden(AgentKind::Act, false));
        assert!(build_delegation_hidden(AgentKind::Act, true));
        assert!(!build_delegation_hidden(AgentKind::Subagent, false));
    }

    /// The `--prompt-file` preamble is a strip target exactly like the
    /// BASE_PROMPT: it must contain the clause (else stripping is a no-op
    /// and the test below proves nothing) and lose every 'build' mention
    /// after stripping.
    #[test]
    fn tool_preamble_build_clause_is_strip_target() {
        assert!(tool_preamble().contains(BUILD_DELEGATION_CLAUSE));
        let stripped = strip_build_delegation(tool_preamble());
        assert!(!stripped.contains("'build'"));
        assert!(stripped.contains("## Tools"));
    }

    /// File-agent fixtures: point the agents root at a tempdir (under
    /// `meta`'s override lock — the override is process-global, so tests
    /// reading it must hold the lock for their whole body).
    fn scoped_agents() -> (tempfile::TempDir, std::sync::MutexGuard<'static, ()>) {
        let dir = tempfile::tempdir().unwrap();
        let guard = meta::tests::OVERRIDE_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        meta::set_agents_dir_override(Some(dir.path().to_path_buf()));
        (dir, guard)
    }

    /// Write `<cat>/<name>/meta.json` pointing `current` at `v`.
    fn write_resource_meta(root: &std::path::Path, cat: &str, name: &str, v: u32) {
        std::fs::create_dir_all(root.join(cat).join(name)).unwrap();
        std::fs::write(
            root.join(cat).join(name).join("meta.json"),
            format!(r#"{{ "name": "{name}", "current": {v}, "history": [{v}] }}"#),
        )
        .unwrap();
    }

    /// Write one shared prompt pack: `prompts/<pname>/v{v}/` with the
    /// given sections plus its pool `meta.json` (`current: v`).
    fn write_prompt_pack(
        root: &std::path::Path,
        pname: &str,
        v: u32,
        soul: Option<&str>,
        how: Option<&str>,
        output: Option<&str>,
    ) {
        let vdir = root.join("prompts").join(pname).join(format!("v{v}"));
        std::fs::create_dir_all(&vdir).unwrap();
        for (file, body) in [("soul", soul), ("how", how), ("output", output)] {
            if let Some(b) = body {
                std::fs::write(vdir.join(format!("{file}.md")), b).unwrap();
            }
        }
        write_resource_meta(root, "prompts", pname, v);
    }

    /// Write one agent reference card naming pool resources (`None` = no
    /// reference for that category).
    fn write_agent_card(
        root: &std::path::Path,
        name: &str,
        prompt: Option<&str>,
        skills: Option<&str>,
        tools: Option<&str>,
        memory: Option<&str>,
    ) {
        std::fs::create_dir_all(root.join(name)).unwrap();
        let meta = serde_json::json!({
            "name": name,
            "current": { "prompt": prompt, "skills": skills, "tools": tools, "memory": memory },
        });
        std::fs::write(root.join(name).join("meta.json"), meta.to_string()).unwrap();
    }

    /// Minimal resolvable file agent: a private shared pool named after
    /// the agent (`prompts/<name>` v1) plus a card referencing it. Each
    /// call gets its OWN pack so section leftovers never leak between
    /// fixtures (write paths only add files, never clear them).
    fn write_file_agent(
        root: &std::path::Path,
        name: &str,
        soul: Option<&str>,
        how: Option<&str>,
        output: Option<&str>,
    ) {
        write_prompt_pack(root, name, 1, soul, how, output);
        write_agent_card(root, name, Some(name), None, None, None);
    }

    /// A file agent resolves with composed prompt, Act/Primary kind, the
    /// first soul line as description, and `ToolFilter::All`.
    #[test]
    fn file_agent_resolves_from_current_prompt_version() {
        let (dir, _g) = scoped_agents();
        write_file_agent(
            dir.path(),
            "myagent",
            Some("Identity: a lean rust reviewer.\nMore soul."),
            Some("How: read, then patch."),
            Some("Output: patch + tests."),
        );
        let a = resolve_agent("myagent").expect("file agent must resolve");
        assert_eq!(a.name, "myagent");
        assert_eq!(a.kind, AgentKind::Act);
        assert!(a.is_primary());
        assert_eq!(a.description, "Identity: a lean rust reviewer.");
        assert_eq!(
            a.prompt,
            "# Soul\nIdentity: a lean rust reviewer.\nMore soul.\n\n# How\nHow: read, then patch.\n\n# Output\nOutput: patch + tests."
        );
        assert!(a.tools.allows("bash") && a.tools.allows("anything"));
        // No soul file ⇒ the description falls back to a stable label.
        write_file_agent(dir.path(), "howonly", None, Some("how body"), None);
        let h = resolve_agent("howonly").expect("how-only agent resolves");
        assert_eq!(h.description, "Custom agent howonly");
    }

    /// The shared-pool property: two agents referencing the SAME prompt
    /// name both resolve through it, and bumping the pool's `current` to
    /// v2 (new version dir + meta, written by hand) updates both at once.
    #[test]
    fn shared_prompt_pool_two_agents_share_and_bump() {
        let (dir, _g) = scoped_agents();
        let root = dir.path();
        write_prompt_pack(root, "shared", 1, Some("v1 soul"), None, None);
        write_agent_card(root, "one", Some("shared"), None, None, None);
        write_agent_card(root, "two", Some("shared"), None, None, None);
        for name in ["one", "two"] {
            assert_eq!(
                resolve_agent(name).unwrap().prompt,
                "# Soul\nv1 soul",
                "{name} must resolve through the shared pack"
            );
        }
        // Hand-write the v2 bump: new version dir + updated pool meta.
        let v2 = root.join("prompts").join("shared").join("v2");
        std::fs::create_dir_all(&v2).unwrap();
        std::fs::write(v2.join("soul.md"), "v2 soul").unwrap();
        std::fs::write(
            root.join("prompts").join("shared").join("meta.json"),
            r#"{ "name": "shared", "current": 2, "history": [1, 2] }"#,
        )
        .unwrap();
        for name in ["one", "two"] {
            assert_eq!(
                resolve_agent(name).unwrap().prompt,
                "# Soul\nv2 soul",
                "{name} must see the v2 bump"
            );
        }
    }

    /// Reference-card degrade paths: no prompt ref ⇒ not resolvable; a
    /// prompt ref to a missing resource (or one with `current: 0`, or a
    /// current version dir that does not exist) ⇒ stale degrade to `None`.
    #[test]
    fn agent_without_prompt_ref_or_stale_refs_degrade() {
        let (dir, _g) = scoped_agents();
        let root = dir.path();
        // Card with only non-prompt refs ⇒ no prompt ⇒ None.
        write_agent_card(root, "noprompt", None, Some("s"), None, None);
        assert!(resolve_agent("noprompt").is_none());
        // Prompt ref to a resource that does not exist ⇒ None.
        write_agent_card(root, "stale", Some("ghost"), None, None, None);
        assert!(resolve_agent("stale").is_none());
        // Pool exists but has no version yet (current: 0) ⇒ None.
        write_resource_meta(root, "prompts", "noversion", 0);
        write_agent_card(root, "zero", Some("noversion"), None, None, None);
        assert!(resolve_agent("zero").is_none());
        // Pool meta points at a version dir that is missing ⇒ None.
        write_resource_meta(root, "prompts", "dangling", 3);
        write_agent_card(root, "dangling", Some("dangling"), None, None, None);
        assert!(resolve_agent("dangling").is_none());
    }

    /// Memory reference: a resolving `current.memory` ref appends a
    /// `# Memory` section (trimmed body) to the composed prompt; no ref,
    /// or a stale ref, appends nothing.
    #[test]
    fn memory_section_appended_when_ref_resolves() {
        let (dir, _g) = scoped_agents();
        let root = dir.path();
        write_prompt_pack(root, "default", 1, Some("soul body"), None, None);
        write_resource_meta(root, "memory", "longterm", 1);
        let memdir = root.join("memory").join("longterm").join("v1");
        std::fs::create_dir_all(&memdir).unwrap();
        std::fs::write(memdir.join("memory.md"), "  prefers small commits.  \n\n").unwrap();
        write_agent_card(root, "withmem", Some("default"), None, None, Some("longterm"));
        write_agent_card(root, "nomem", Some("default"), None, None, None);
        write_agent_card(root, "stalemem", Some("default"), None, None, Some("ghost"));
        let with = resolve_agent("withmem").unwrap().prompt;
        assert!(with.ends_with("# Soul\nsoul body\n\n# Memory\nprefers small commits."));
        assert!(!resolve_agent("nomem").unwrap().prompt.contains("# Memory"));
        assert!(
            !resolve_agent("stalemem").unwrap().prompt.contains("# Memory"),
            "a stale memory ref must append nothing"
        );
    }

    /// Missing files degrade: sections are optional, but an agent with no
    /// readable section at all (missing, or a prompt version that does not
    /// exist, or no prompt reference in the card) is not a real agent →
    /// `None`.
    #[test]
    fn file_agent_missing_sections_and_versions_degrade() {
        let (dir, _g) = scoped_agents();
        write_file_agent(dir.path(), "partial", None, Some("only how"), None);
        assert_eq!(resolve_agent("partial").unwrap().prompt, "# How\nonly how");
        // Pool version exists but holds no section files.
        write_prompt_pack(dir.path(), "bare", 1, None, None, None);
        write_agent_card(dir.path(), "empty", Some("bare"), None, None, None);
        assert!(resolve_agent("empty").is_none());
        // Blank-only sections are as good as missing.
        write_file_agent(dir.path(), "blank", Some("  "), None, None);
        assert!(resolve_agent("blank").is_none());
    }

    /// Builtin names always win over same-named file agents, and corrupt
    /// file agents fall back to `None` (builtin-fallback behavior) without
    /// touching traversal paths.
    #[test]
    fn builtin_wins_and_corrupt_file_agents_fall_back() {
        let (dir, _g) = scoped_agents();
        // A file agent (illegally) named like a builtin must not shadow it.
        write_file_agent(dir.path(), "act", Some("impostor"), None, None);
        assert!(resolve_agent("act")
            .unwrap()
            .prompt
            .contains("You are OpenCoder"));
        // Corrupt agent card → silent None.
        write_file_agent(dir.path(), "corrupt", Some("s"), None, None);
        std::fs::write(dir.path().join("corrupt/meta.json"), "{ not json").unwrap();
        assert!(resolve_agent("corrupt").is_none());
        // Corrupt PROMPT POOL meta → the shared resource degrades → None.
        write_agent_card(dir.path(), "badpool", Some("broken"), None, None, None);
        std::fs::create_dir_all(dir.path().join("prompts").join("broken")).unwrap();
        std::fs::write(dir.path().join("prompts/broken/meta.json"), "{ not json").unwrap();
        assert!(resolve_agent("badpool").is_none());
        // Path safety: traversal strings never reach the filesystem layer.
        assert!(resolve_agent("../evil").is_none());
        assert!(resolve_agent("a/b").is_none());
        assert!(resolve_agent("").is_none());
    }

    /// All four tiers of [`effective_default_agent`]: CLI > active file
    /// agent > config default > `"act"`.
    #[test]
    fn effective_default_agent_priority_tiers() {
        let (dir, _g) = scoped_agents();
        let cfg = Config::default();
        // Tier 4: nothing set anywhere.
        assert_eq!(effective_default_agent(None, &cfg), "act");
        // Tier 3: config default (non-empty) beats "act".
        let mut cfg_d = Config::default();
        cfg_d.agent.default = "plan".into();
        assert_eq!(effective_default_agent(None, &cfg_d), "plan");
        // Blank config default is skipped → tier 4.
        let mut cfg_blank = Config::default();
        cfg_blank.agent.default = "  ".into();
        assert_eq!(effective_default_agent(None, &cfg_blank), "act");
        // Tier 2: the active file agent beats config.
        write_file_agent(dir.path(), "mine", Some("soul line"), None, None);
        meta::set_active_agent(Some("mine")).unwrap();
        assert_eq!(effective_default_agent(None, &cfg_d), "mine");
        // A stale marker deactivates silently → tier 3 again.
        std::fs::remove_dir_all(dir.path().join("mine")).unwrap();
        assert_eq!(effective_default_agent(None, &cfg_d), "plan");
        // Tier 1: the CLI override beats everything (and blank is skipped).
        assert_eq!(effective_default_agent(Some("cli"), &cfg_d), "cli");
        assert_eq!(effective_default_agent(Some("  "), &cfg_d), "plan");
    }
}
