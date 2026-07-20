use opencoder_core::{message::now_ms, CapabilitiesConfig, Message};
use std::path::{Path, PathBuf};

pub fn build_system(
    agent: &opencoder_core::Agent,
    working_dir: &Path,
    skill_prompt: Option<&str>,
    caps: &CapabilitiesConfig,
) -> Message {
    let mut text = agent.prompt.clone();
    // Hide the 'tools' umbrella subagent advertisement when the capability is
    // off, so the model is never told the `tools` subagent exists.
    if !caps.tools_subagent_enabled() {
        text = opencoder_core::strip_tools_subagent_ad(&text);
    }

    if let Some(instructions) = load_instructions(working_dir) {
        text.push_str("\n\n## Project instructions\n");
        text.push_str(&instructions);
    }

    let env = environment_block(working_dir);
    text.push_str("\n\n");
    text.push_str(&env);

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

/// Load and concatenate project instruction files (AGENTS.md) from up to
/// three locations, in increasing priority:
///   1. Global:    `~/.opencoder/AGENTS.md`
///   2. Git root:  `<git_root>/AGENTS.md` (found by walking up from working_dir)
///   3. Working:   `<working_dir>/AGENTS.md`
///
/// Filenames are matched case-insensitively. Missing or unreadable files are
/// silently skipped. Duplicate directories (e.g. git root == working_dir) are
/// loaded only once. Returns `None` when no file was found.
fn load_instructions(working_dir: &Path) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".opencoder"));
    }
    if let Some(root) = find_git_root(working_dir) {
        candidates.push(root);
    }
    candidates.push(working_dir.to_path_buf());

    for dir in candidates {
        let canon = dir.canonicalize().unwrap_or_else(|_| dir.clone());
        if seen.iter().any(|s| s == &canon) {
            continue;
        }
        seen.push(canon);

        if let Some(path) = find_agents_md(&dir) {
            if let Ok(content) = std::fs::read_to_string(&path) {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_string());
                }
            }
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

/// Load ONLY the global `~/.opencoder/AGENTS.md` instructions — the ambient,
/// always-on baseline file. Unlike `load_instructions` (which merges global,
/// git-root, and working-dir files into the system prompt), this returns just
/// the global portion so callers can exclude it from the per-session
/// context-token accounting.
///
/// The global file still ships in the system prompt (see `build_system`); only
/// its token *budget* is treated as "free" baseline context, so a large global
/// instructions file does not consume the conversation window or inflate the
/// TUI context meter.
///
/// Returns the trimmed content, or `None` when the file is absent, unreadable,
/// empty, or when the global dir coincides with `working_dir` (in that last
/// case the same content is loaded/counted as a local instruction).
pub fn global_instructions_text(working_dir: &Path) -> Option<String> {
    let home = dirs::home_dir()?;
    let dir = home.join(".opencoder");
    // When the global dir is the working dir, the same file is loaded as a
    // local instruction and must stay counted — bail out so we never subtract.
    let canon = dir.canonicalize().unwrap_or_else(|_| dir.clone());
    let wd_canon = working_dir
        .canonicalize()
        .unwrap_or_else(|_| working_dir.to_path_buf());
    if canon == wd_canon {
        return None;
    }
    let path = find_agents_md(&dir)?;
    let content = std::fs::read_to_string(&path).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Case-insensitive search for an `AGENTS.md` file inside `dir`.
fn find_agents_md(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        if entry.file_name().eq_ignore_ascii_case("AGENTS.md") {
            let path = entry.path();
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

/// Walk up from `start` to find the nearest directory containing a `.git`
/// marker (file or directory). Returns the containing directory.
fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut current = Some(start);
    while let Some(dir) = current {
        if dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    None
}

pub fn environment_block(working_dir: &Path) -> String {
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
