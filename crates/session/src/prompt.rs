use opencoder_core::{message::now_ms, Message};

pub fn build_system(
    agent: &opencoder_core::Agent,
    working_dir: &std::path::Path,
    skill_prompt: Option<&str>,
) -> Message {
    let env = environment_block(working_dir);
    let mut text = format!("{}\n\n{}", agent.prompt, env);
    if let Some(skill) = skill_prompt {
        let trimmed = skill.trim();
        if !trimmed.is_empty() {
            // Appended last so an active skill is the highest-priority
            // instruction in the system prompt.
            text.push_str("\n\n## Active skill\n");
            text.push_str(trimmed);
        }
    }
    Message::system("system", text)
}

pub fn environment_block(working_dir: &std::path::Path) -> String {
    let platform = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let date = chrono::Utc::now().format("%a %b %d %Y").to_string();
    let mut s = String::new();
    s.push_str("# Environment\n");
    s.push_str(&format!("- Working directory: {}\n", working_dir.display()));
    s.push_str("- Stay within the working directory: you may work in its subdirectories, but do not access or modify anything outside it.\n");
    s.push_str(&format!("- Platform: {platform}-{arch}\n"));
    s.push_str(&format!("- Date: {date}\n"));
    s.push_str("- You have file system and shell access via your tools. Run tools in parallel when independent.\n");
    s
}

/// System prompt for the compaction summarizer model.
/// Instructs it to act as an anchored context summarization assistant
/// that produces a structured Markdown summary, incrementally updating a
/// previous summary when one is provided.
pub fn compaction_system_prompt() -> &'static str {
    "You are an anchored context summarization assistant for coding sessions.\n\
     \n\
     Summarize only the conversation history you are given. The newest turns may be kept verbatim outside your summary, so focus on the older context that still matters for continuing the work.\n\
     \n\
     If the prompt includes a <previous-summary> block, treat it as the current anchored summary. Update it with the new history by preserving still-true details, removing stale details, and merging in new facts.\n\
     \n\
     Always follow the exact output structure requested by the user prompt. Keep every section, preserve exact file paths and identifiers when known, and prefer terse bullets over paragraphs.\n\
     \n\
     Do not answer the conversation itself. Do not mention that you are summarizing, compacting, or merging context. Respond in the same language as the conversation."
}

/// User prompt for the compaction summarizer. Produces a structured Markdown
/// summary. When `previous_summary` is provided, the summarizer incrementally
/// updates it rather than writing from scratch.
pub fn compaction_user_prompt(previous_summary: Option<&str>) -> String {
    let header = match previous_summary {
        Some(prev) => format!(
            "Update the anchored summary below using the conversation history above.\n\
             Preserve still-true details, remove stale details, and merge in the new facts.\n\
             <previous-summary>\n{prev}\n</previous-summary>"
        ),
        None => "Create a new anchored summary from the conversation history.".to_string(),
    };

    format!(
        "{header}\n\
         \n\
         Output exactly the Markdown structure shown inside <template> and keep the section order unchanged. Do not include the <template> tags in your response.\n\
         <template>\n\
         ## Objective\n\
         - [one or two brief sentences describing what the user is trying to accomplish]\n\
         \n\
         ## Important Details\n\
         - [constraints/preferences, decisions and why, important facts/assumptions, exact context needed to continue, or \"(none)\"]\n\
         \n\
         ## Work State\n\
         ### Completed\n\
         - [finished work, verified facts, or changes made; otherwise \"(none)\"]\n\
         \n\
         ### Active\n\
         - [current work, partial changes, or investigation state; otherwise \"(none)\"]\n\
         \n\
         ### Blocked\n\
         - [blockers, failing commands, or unknowns; otherwise \"(none)\"]\n\
         \n\
         ## Next Move\n\
         1. [immediate concrete action, or \"(none)\"]\n\
         2. [next action if known, or \"(none)\"]\n\
         \n\
         ## Relevant Files\n\
         - [file or directory path: why it matters, or \"(none)\"]\n\
         </template>\n\
         \n\
         Rules:\n\
         - Keep every section, even when empty.\n\
         - Use terse bullets, not prose paragraphs.\n\
         - Preserve exact file paths, symbols, commands, error strings, URLs, and identifiers when known.\n\
         - Do not mention the summary process or that context was compacted."
    )
}

pub fn _ts() -> i64 {
    now_ms()
}
