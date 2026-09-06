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
    AgentMeta, AgentReferences, AgentRefs,
};
pub use resource::{
    active_skill_roots, active_tools_dirs, agent_skill_roots, agent_tools_dirs, all_tools_dirs,
    category_dir, list_resources, read_resource_meta, resource_current_version_dir,
    resource_version_dir, tools_paths, validate_resource_name, ResourceMeta, AGENT_CATEGORIES,
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

pub mod scope;
#[cfg(test)]
mod tests;
