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
    Sandbox,
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
            name: "sandbox".into(),
            kind: AgentKind::Sandbox,
            mode: AgentMode::Primary,
            description: "Read-only sandbox agent. Explores and answers questions; mutating operations are intercepted.".into(),
            prompt: base_prompt_sandbox(),
            tools: ToolFilter::Allow(vec![
                "bash".into(), "task".into(),
                // Structured clarification: the sandbox agent asks over
                // assuming when an unstated assumption would shape the work.
                // Repo/rules/test facts are looked up, not asked.
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
/// `base_prompt_sandbox` for build-stripping in sandbox mode.
pub fn tool_preamble() -> &'static str {
    "## Tools
- You have two tools: bash (terminal ops: git, builds, tests, running scripts) and task (to spawn subagents).
- For file operations, delegate to subagents: use 'explore' (read-only) for investigation, 'build' (full tools) for implementation.
- Run tool calls in parallel when none needs the other's output; otherwise run sequentially. You MAY emit multiple `task` blocks in a single response -- independent subagents dispatched this way run concurrently, so prefer batching independent investigations.
- Keep responses concise and friendly. Do not dump large files; reference paths only.
- When a tool errors, read the error, fix the approach, and retry; do not loop on the same failing command.
"
}

pub fn base_prompt_sandbox() -> String {
    // Sandbox mode must not advertise the 'build' subagent: strip the build
    // delegation clause from the shared base prompt before appending the
    // sandbox suffix. Act mode keeps the full BASE_PROMPT unchanged.
    let base = BASE_PROMPT.replace(", 'build' (full tools) for implementation", "");
    format!("{base}\n\n{}", SANDBOX_SUFFIX)
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

const SANDBOX_SUFFIX: &str = "\
SANDBOX mode (read-only): no edits/writes; mutating bash (file-writing redirects, rm, mv, git push, pip install, ...) is intercepted. \
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

    /// Guards the `.replace()` in `base_prompt_sandbox()`: if BASE_PROMPT's
    /// wording ever drifts so the replace becomes a no-op, the build subagent
    /// advertisement silently leaks into the sandbox prompt. These assertions
    /// fail loudly instead.
    #[test]
    fn sandbox_prompt_strips_build_subagent_advertisement() {
        // The exact substring targeted by `.replace()` in base_prompt_sandbox().
        // If this assertion fails, BASE_PROMPT has changed — update the
        // `.replace()` call to match the new wording.
        let replace_target = ", 'build' (full tools) for implementation";
        assert!(
            base_prompt_act().contains(replace_target),
            "BASE_PROMPT no longer contains the '.replace()' target substring \
             {replace_target:?}. Update the .replace() call in base_prompt_sandbox()."
        );

        let sandbox = base_prompt_sandbox();

        // Safety property: the sandbox prompt must not advertise 'build'.
        assert!(
            !sandbox.contains("'build' (full tools)"),
            "sandbox prompt must not advertise the 'build' subagent, got: {sandbox}"
        );

        // Sanity: the 'explore' advertisement must survive (the replace should
        // only strip the build clause, not the entire delegation line).
        assert!(
            sandbox.contains("'explore' (read-only)"),
            "sandbox prompt must still advertise 'explore', got: {sandbox}"
        );
    }

    /// `question` is allowlisted for the two primary agents that may surface
    /// clarification prompts: `sandbox` asks over assuming, and `act` is
    /// allowlisted here (runtime visibility is gated elsewhere). Subagents
    /// never ask -- zero schema token cost. Structural guard (rules/01)
    /// against filter drift.
    #[test]
    fn question_tool_is_sandbox_and_act_only() {
        for name in ["sandbox", "act"] {
            let a = resolve_agent(name).expect("primary agent registered");
            assert!(a.tools.allows("question"), "{name} must allow 'question'");
        }
        for other in ["explore", "build", "command", "workflow"] {
            let a = resolve_agent(other).expect("agent registered");
            assert!(
                !a.tools.allows("question"),
                "{other} must not allow 'question'"
            );
        }
    }

    /// The sandbox prompt is a general read-only preamble, NOT a plan
    /// producer: the old Goal/TODO/Verify/Risks/Align template and the
    /// "switch to act mode" handoff are gone. Pins the read-only constraints
    /// and the ask-over-assume guidance (batched questions allowed;
    /// repo/rules/test facts looked up, not asked).
    #[test]
    fn sandbox_prompt_is_read_only_with_question_guidance() {
        let sandbox = base_prompt_sandbox();

        // Read-only constraints survive the rename.
        assert!(
            sandbox.contains("read-only"),
            "sandbox prompt must state its read-only constraints, got: {sandbox}"
        );
        assert!(
            sandbox.contains("mutating bash"),
            "sandbox prompt must note intercepted mutating bash, got: {sandbox}"
        );

        // Question-tool clarification guidance.
        assert!(
            sandbox.contains("prefer asking over assuming"),
            "sandbox prompt must default to asking instead of assuming, got: {sandbox}"
        );
        assert!(
            sandbox.contains("you may ask several in one turn"),
            "sandbox prompt must advertise batched clarification, got: {sandbox}"
        );
        assert!(
            sandbox.contains("looked up first, not asked"),
            "sandbox prompt must defer repo/rules/test facts to lookup, got: {sandbox}"
        );

        // Plan-template semantics are gone.
        assert!(
            !sandbox.contains("Goal / TODO / Verify / Risks / Align"),
            "sandbox prompt must not require the plan template sections, got: {sandbox}"
        );
        assert!(
            !sandbox.contains("act mode"),
            "sandbox prompt must not hand off to a plan/act mode switch, got: {sandbox}"
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
}
