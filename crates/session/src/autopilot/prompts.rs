//! Prompt templates for the autopilot phases. Kept as plain functions (no
//! allocation beyond `format!`) so they are trivially stable and diffable.

/// Injected at the start of the PLAN phase. Anchors the agent to the original
/// goal and asks for a concrete plan; the plan turns are legitimate work
/// records that stay in the main transcript for VERIFY to inspect.
pub fn continuation_prompt(goal: &str) -> String {
    format!(
        "Autopilot PLAN phase (iteration in progress).\n\n\
         Goal: {goal}\n\n\
         Review the current state of work toward this goal. Decide what concrete \
         next steps are needed and produce a focused plan. Do NOT redo work that \
         is already complete; identify the highest-value remaining actions. \
         Keep the plan short and actionable.",
    )
}

/// Injected at the start of the ACT phase. Context is carried over from PLAN
/// (no handoff reset) so the execution agent sees the full conversation.
pub fn execute_prompt() -> String {
    "Autopilot ACT phase. Execute the plan you just produced using your tools. \
     Make real progress toward the goal. When you have done as much as you \
     productively can in this turn, stop and summarize what changed."
        .to_string()
}

/// System prompt for the isolated shadow VERIFY one-shot. This message is part
/// of the ephemeral snapshot and is NEVER persisted into the main transcript.
pub fn verify_system_prompt() -> String {
    "You are a strict verification judge. You are given the full transcript of \
     an autonomous coding session working toward a goal. Decide whether the \
     goal is fully achieved. Reply with a SINGLE token: 'yes' if MORE work is \
     still needed, 'no' if the task is COMPLETE. Output nothing else."
        .to_string()
}

/// User turn appended to the snapshot for the VERIFY one-shot, naming the goal
/// so the judgement is anchored to the original intent.
pub fn verify_user_prompt(goal: &str) -> String {
    format!(
        "Goal: {goal}\n\n\
         Based on the transcript above, is MORE work needed to fully achieve \
         this goal? Reply with a single token: 'yes' (more work) or 'no' \
         (complete).",
    )
}
