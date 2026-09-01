use serde::{Deserialize, Serialize};

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

pub fn resolve_agent(name: &str) -> Option<Agent> {
    builtin_agents().into_iter().find(|a| a.name == name)
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
                // Structured clarification: the plan agent asks over
                // assuming when an unstated assumption would shape the work.
                // Repo/rules/test facts are looked up, not asked.
                // Latent: surfaced only while the task-plan skill is active.
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
                "read".into(), "search".into(), "ls".into(),
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
     reference material. You have read, search, and ls tools to check files when the snapshot is \
     not enough. You CANNOT edit or write files and must never claim any change was made. \
     Answer concisely and progress-oriented: what is done, what is in flight, what comes next."
        .to_string()
}

const PLAN_SUFFIX: &str = "\
PLAN mode (read-only): no edits/writes and no implementation execution. Every state-changing tool attempt (including writes under /tmp) is intercepted and returned in context. If blocked, do not retry or look for another write path; focus on analysis and output a plan only. \
Investigate via 'explore' subagents. \
When an unstated assumption would shape your work, resolve it via the `question` tool -- prefer asking over assuming (you may ask several in one turn). Facts the repo, rules/, or tests can answer must be looked up first, not asked.";

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

    /// `question` is allowlisted for the two primary agents that may surface
    /// clarification prompts: `plan` asks over assuming, and `act` is
    /// allowlisted here (runtime visibility is gated elsewhere). Subagents
    /// never ask -- zero schema token cost. Structural guard (rules/01)
    /// against filter drift.
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
    /// (read/search/ls), never mutating or delegation tools. The sidecar
    /// answers questions about the main task from a context snapshot; it must
    /// never be able to change state.
    #[test]
    fn sidecar_observer_is_read_only() {
        let sidecar = resolve_agent("sidecar").expect("sidecar agent registered");
        assert_eq!(sidecar.kind, AgentKind::Subagent);
        assert_eq!(sidecar.mode, AgentMode::Subagent);
        for allowed in &["read", "search", "ls"] {
            assert!(
                sidecar.tools.allows(allowed),
                "sidecar must allow '{allowed}'"
            );
        }
        for blocked in &["bash", "edit", "write", "task", "question"] {
            assert!(
                !sidecar.tools.allows(blocked),
                "sidecar (read-only) must not allow '{blocked}'"
            );
        }
        // The prompt states the observer contract: snapshot-in, progress-out,
        // and no modification claims.
        let prompt = sidecar.prompt;
        assert!(prompt.contains("sidecar observer"), "got: {prompt}");
        assert!(prompt.contains("CANNOT edit or write"), "got: {prompt}");
    }

    /// The plan prompt requires a focused plan without reviving the old rigid
    /// Goal/TODO/Verify/Risks/Align template or an automatic act handoff.
    #[test]
    fn plan_prompt_is_read_only_with_question_guidance() {
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

        // No question-tool advertisement in the base prompt: clarification
        // guidance lives only in the task-plan skill body. (Generic prose
        // like "without asking questions" is fine — only the backticked
        // tool name or an explicit tool mention advertises the schema.)
        assert!(
            plan.contains("prefer asking over assuming"),
            "plan prompt must default to asking instead of assuming, got: {plan}"
        );
        assert!(
            plan.contains("you may ask several in one turn"),
            "plan prompt must advertise batched clarification, got: {plan}"
        );
        assert!(
            plan.contains("looked up first, not asked"),
            "plan prompt must defer repo/rules/test facts to lookup, got: {plan}"
        );

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
}
