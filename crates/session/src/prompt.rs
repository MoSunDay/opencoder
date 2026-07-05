use opencode_core::{message::now_ms, Message};

pub fn build_system(
    agent: &opencode_core::Agent,
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
    s.push_str(&format!("- Platform: {platform}-{arch}\n"));
    s.push_str(&format!("- Date: {date}\n"));
    s.push_str("- You have file system and shell access via your tools. Run tools in parallel when independent.\n");
    s
}

pub fn compaction_prompt() -> String {
    "You are a conversation summarizer. Summarize the conversation above concisely, preserving: the user's goal, decisions made, files created/edited (with paths), commands run and their outcomes, and the current next step. Output only the summary, no preamble.".to_string()
}

pub fn plan_to_act_note() -> String {
    "Your operational mode has changed from plan to act. You are no longer in read-only mode. You may now edit files and run commands. Execute the approved plan.".to_string()
}

pub fn _ts() -> i64 { now_ms() }
