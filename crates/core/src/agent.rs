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
            name: "tools".into(),
            kind: AgentKind::Subagent,
            mode: AgentMode::Subagent,
            description: "Umbrella subagent for optional capabilities: browser (web_fetch, web_search) and computer_use, plus read-only filesystem tools (read, glob, grep, ls). Use for web research or computer-use tasks.".into(),
            prompt: base_prompt_tools(),
            tools: ToolFilter::Allow(vec![
                "web_fetch".into(), "web_search".into(), "computer_use".into(),
                "read".into(), "glob".into(), "grep".into(), "ls".into(),
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
    // Plan mode must not advertise the 'build' subagent: strip the build
    // delegation clause from the shared base prompt before appending the plan
    // suffix. Act mode keeps the full BASE_PROMPT unchanged.
    let base = BASE_PROMPT.replace(", 'build' (full tools) for implementation", "");
    format!("{base}\n\n{}", PLAN_SUFFIX)
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

pub fn base_prompt_tools() -> String {
    "You are the tools subagent: the home of optional capabilities — browser (web_fetch, web_search) \
     and computer-use (computer_use) — plus read-only filesystem tools (read, glob, grep, ls). \
     Complete the specific task delegated to you. Browser and computer-use tools are only present when \
     the user has enabled the corresponding capability, so fall back to read-only investigation if a \
     tool is unavailable. Do not ask questions; infer reasonable defaults and proceed. \
     After finishing, briefly state what you did and return any fetched content or action trace."
        .to_string()
}

const PLAN_SUFFIX: &str = "\
PLAN mode (read-only): no edits/writes; mutating bash (file-writing redirects, rm, mv, git push, pip install, ...) is intercepted. \
Investigate via 'explore' subagents. \
Output an actionable plan the user reviews before switching to act mode; ask clarifying questions first if anything is ambiguous -- do not assume intent. \
The plan MUST have these sections: Goal / TODO / Verify / Risks / Align.";

const BASE_PROMPT: &str = "\
You are OpenCoder, a high-performance coding agent in a terminal.

## How to work
- Default to doing the work without asking questions. Infer missing details by reading the codebase and following existing conventions.
- You have two tools: bash (for terminal ops: git, builds, tests, running scripts) and task (to spawn subagents).
- For file operations, delegate to subagents: use 'explore' (read-only) for investigation, 'build' (full tools) for implementation.
- For browser (web fetch/search) or computer-use tasks, delegate to the 'tools' subagent.
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

    /// Guards the `.replace()` in `base_prompt_plan()`: if BASE_PROMPT's wording
    /// ever drifts so the replace becomes a no-op, the build subagent advertisement
    /// silently leaks into the plan prompt. These assertions fail loudly instead.
    #[test]
    fn plan_prompt_strips_build_subagent_advertisement() {
        // The exact substring targeted by `.replace()` in base_prompt_plan().
        // If this assertion fails, BASE_PROMPT has changed — update the
        // `.replace()` call to match the new wording.
        let replace_target = ", 'build' (full tools) for implementation";
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

    /// The `tools` umbrella subagent must resolve and carry exactly the
    /// capability + read-only filesystem tools (browser/computer-use are
    /// runtime-gated by config + the `browser` cargo feature).
    #[test]
    fn tools_subagent_is_registered_with_capability_tools() {
        let tools_agent = resolve_agent("tools").expect("tools subagent registered");
        assert_eq!(tools_agent.mode, AgentMode::Subagent);
        for required in ["web_fetch", "web_search", "computer_use", "read", "glob", "grep", "ls"] {
            assert!(
                tools_agent.tools.allows(required),
                "tools subagent must allow '{required}'"
            );
        }
        // tools is plan-visible: act and plan prompts both advertise it.
        assert!(base_prompt_act().contains("'tools' subagent"));
        assert!(base_prompt_plan().contains("'tools' subagent"));
    }
}
