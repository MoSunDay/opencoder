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

/// Fallback execute prompt for the ACT phase, injected only when the plan→act
/// handoff finds no plan to focus the transcript. The normal ACT path resets
/// the transcript via handoff (whose message carries execution directives) and
/// does not inject this prompt.
pub fn execute_prompt() -> String {
    "Autopilot ACT phase. Execute the plan you just produced using your tools. \
     Make real progress toward the goal. When you have done as much as you \
     productively can in this turn, stop and summarize what changed."
        .to_string()
}

/// System prompt for the isolated shadow VERIFY one-shot. This message is part
/// of the ephemeral snapshot and is NEVER persisted into the main transcript.
///
/// The question is phrased positively ("is the goal fully achieved?") so the
/// judge's instinctive "yes, done" maps to `Complete`. Asking "is more work
/// needed?" instead biased a chatty judge toward perpetual `MoreWork`.
pub fn verify_system_prompt() -> String {
    "You are a strict verification judge. You are given the full transcript of \
     an autonomous coding session working toward a goal. Decide whether the \
     goal is fully achieved. Reply with a SINGLE token: 'yes' if the goal is \
     ACHIEVED (task complete), 'no' if more work is still needed. Output \
     nothing else."
        .to_string()
}

/// User turn appended to the snapshot for the VERIFY one-shot, naming the goal
/// so the judgement is anchored to the original intent.
pub fn verify_user_prompt(goal: &str) -> String {
    format!(
        "Goal: {goal}\n\n\
         Based on the transcript above, is the goal fully achieved? Answer \
         with EXACTLY one token: 'yes' (achieved / complete) or 'no' (more \
         work needed) — nothing else. Parsing is strict: any qualifier, \
         explanation or extra word makes the answer count as no verdict.",
    )
}

/// User turn for the one-shot review pass (`autopilot.mode = "review"`).
/// Anchors the review to the original goal; the review skill body rides in
/// the system prompt (activated by `activate_review_skill`).
pub fn review_prompt(goal: &str) -> String {
    format!(
        "Review the work completed toward this goal: {goal}\n\n\
         Review the current state of the work — correctness, completeness, and \
         any defects or risks. Do NOT redo or extend the work; produce a \
         focused review of what was done. Keep it short and actionable."
    )
}
