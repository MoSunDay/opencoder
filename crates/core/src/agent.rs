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
    Plan,
    Subagent,
    Command,
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
            tools: ToolFilter::Allow(vec!["bash".into(), "task".into()]),
        },
        Agent {
            name: "plan".into(),
            kind: AgentKind::Plan,
            mode: AgentMode::Primary,
            description: "Read-only planning agent. Explores via subagents and produces a plan.".into(),
            prompt: base_prompt_plan(),
            tools: ToolFilter::Allow(vec![
                "bash".into(), "task".into(),
            ]),
        },
        Agent {
            name: "explore".into(),
            kind: AgentKind::Subagent,
            mode: AgentMode::Subagent,
            description: "Read-only subagent for exploring codebases: find files, search code, answer questions. Cannot modify files.".into(),
            prompt: base_prompt_explore(),
            tools: ToolFilter::Allow(vec![
                "read".into(), "glob".into(), "grep".into(), "ls".into(), "bash".into(),
            ]),
        },
        Agent {
            name: "build".into(),
            kind: AgentKind::Subagent,
            mode: AgentMode::Subagent,
            description: "Implementation subagent with full file tools: read, write, edit, bash, glob, grep. Use for making code changes.".into(),
            prompt: base_prompt_build(),
            tools: ToolFilter::Allow(vec![
                "read".into(), "write".into(), "edit".into(), "bash".into(),
                "glob".into(), "grep".into(), "ls".into(),
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
    ]
}

pub fn base_prompt_act() -> String {
    BASE_PROMPT.to_string()
}

pub fn base_prompt_plan() -> String {
    format!("{BASE_PROMPT}\n\n{}", PLAN_SUFFIX)
}

pub fn base_prompt_explore() -> String {
    "You are a read-only exploration subagent. Your job is to investigate the codebase and report findings. \
     You have read, glob, grep, ls, and bash (read-only) tools. You CANNOT edit or write files. \
     Complete the specific task delegated to you, then return a concise summary of your findings. \
     Do not ask questions; infer reasonable defaults and proceed."
        .to_string()
}

pub fn base_prompt_build() -> String {
    "You are an implementation subagent with full file tools. Complete the specific task delegated to you: \
     read code, make edits, write new files, run bash commands, and verify your work. \
     Do not ask questions; infer reasonable defaults and proceed. \
     After finishing, briefly state what you changed and the key file paths."
        .to_string()
}

const PLAN_SUFFIX: &str = "\
You are in PLAN mode: read-only. You cannot edit, write, or run mutating bash commands. \
Bash write commands (redirects, rm, mv, git push, pip install, etc.) will be intercepted and rejected. \
Use the task tool to spawn 'explore' subagents for codebase investigation. \
Produce a clear, actionable plan as text. The user will review it and switch to act mode to execute. \
If any requirement is ambiguous or involves trade-offs that affect the user, ask clarifying questions \
BEFORE finalizing the plan. Do not assume user intent.";

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
