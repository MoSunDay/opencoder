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
    pub max_steps: u32,
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
            description: "Default execution agent. Can read, edit, and run commands to complete tasks.".into(),
            prompt: base_prompt_act(),
            tools: ToolFilter::All,
            max_steps: 50,
        },
        Agent {
            name: "plan".into(),
            kind: AgentKind::Plan,
            mode: AgentMode::Primary,
            description: "Read-only planning agent. Explores and writes a plan file, then exits plan mode.".into(),
            prompt: base_prompt_plan(),
            tools: ToolFilter::Allow(vec![
                "read".into(), "glob".into(), "grep".into(), "bash".into(), "ls".into(),
                "task".into(), "plan_exit".into(),
            ]),
            max_steps: 30,
        },
        Agent {
            name: "subagent".into(),
            kind: AgentKind::Subagent,
            mode: AgentMode::Subagent,
            description: "A restricted subagent spawned via the task tool to explore or implement in isolation.".into(),
            prompt: base_prompt_subagent(),
            tools: ToolFilter::All,
            max_steps: 30,
        },
        Agent {
            name: "command".into(),
            kind: AgentKind::Command,
            mode: AgentMode::Primary,
            description: "One-shot single-turn agent. Runs a single prompt to completion without interactive follow-up.".into(),
            prompt: base_prompt_act(),
            tools: ToolFilter::All,
            max_steps: 50,
        },
    ]
}

pub fn base_prompt_act() -> String {
    BASE_PROMPT.to_string()
}

pub fn base_prompt_plan() -> String {
    format!("{BASE_PROMPT}\n\n{}", PLAN_SUFFIX)
}

pub fn base_prompt_subagent() -> String {
    "You are a focused subagent. Complete the specific task delegated to you, then stop. Do not ask questions; infer reasonable defaults and proceed.".to_string()
}

const PLAN_SUFFIX: &str = "\
You are in PLAN mode: read-only. Do NOT edit, write, or run mutating commands. \
Explore the codebase using read/glob/grep/bash (read-only), optionally spawn subagents to explore in parallel, \
then write the plan to .opencode/plans/<plan>.md using the plan_exit tool (which is the ONLY write action allowed). \
After writing the plan, call the plan_exit tool to switch to act mode.";

const BASE_PROMPT: &str = "\
You are OpenCoder, a high-performance coding agent in a terminal.

## How to work
- Default to doing the work without asking questions. Infer missing details by reading the codebase and following existing conventions.
- Prefer specialized tools over shell for file operations: read to view, edit to modify, write only for new files. Use bash for terminal ops (git, builds, tests, running scripts).
- Run tool calls in parallel when none needs the other's output; otherwise run sequentially.
- Keep responses concise and friendly. Do not dump large files you wrote; reference paths only.
- Only add comments when necessary.

## Editing
- Default to ASCII. Match existing file style.
- Never revert changes you did not make. Do not amend commits unless asked. Avoid destructive git commands (reset --hard, checkout --) unless explicitly requested.

## Tool results
- When a tool errors, read the error, fix the approach, and retry; do not loop on the same failing command.
- After finishing, briefly state what you did and the key files, and suggest logical next steps (tests, build, commit).
";
