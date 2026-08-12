use opencoder_core::{config::McpServerConfig, message::now_ms, AgentKind, Message};
use std::path::{Path, PathBuf};

pub fn build_system(
    agent: &opencoder_core::Agent,
    working_dir: &Path,
    skill_prompt: Option<&str>,
    mcp_block: Option<&str>,
) -> Message {
    let mut text = agent.prompt.clone();

    if let Some(instructions) = load_instructions(working_dir) {
        text.push_str("\n\n## Project instructions\n");
        text.push_str(&instructions);
    }

    let env = environment_block(working_dir, agent.kind);
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

    if let Some(mcp) = mcp_block {
        let trimmed = mcp.trim();
        if !trimmed.is_empty() {
            text.push_str("\n\n");
            text.push_str(trimmed);
        }
    }

    Message::system("system", text)
}

/// Build the `## MCP Servers` system-prompt section.
///
/// Returns `None` when there are no enabled servers (zero behaviour change
/// for sessions without MCP). The section lists each enabled server with its
/// transport (stdio `command args` or SSE `url`) so the model is aware of
/// the available MCP servers.
pub fn mcp_section(servers: &[(String, &McpServerConfig)]) -> Option<String> {
    if servers.is_empty() {
        return None;
    }
    let mut lines = String::from("## MCP Servers\n");
    lines.push_str(
        "The following MCP (Model Context Protocol) servers are enabled. \
         They provide additional tools and resources.",
    );
    for (name, cfg) in servers {
        lines.push_str("\n\n### ");
        lines.push_str(name);
        match (&cfg.command, &cfg.url) {
            (Some(cmd), _) => {
                lines.push_str(" (stdio)");
                lines.push_str("\n- command: `");
                lines.push_str(cmd);
                for a in &cfg.args {
                    lines.push(' ');
                    lines.push_str(a);
                }
                lines.push('`');
            }
            (None, Some(url)) => {
                lines.push_str(" (sse)");
                lines.push_str("\n- url: `");
                lines.push_str(url);
                lines.push('`');
            }
            (None, None) => {
                lines.push_str("\n- (no transport configured)");
            }
        }
        if !cfg.env.is_empty() {
            lines.push_str(&format!("\n- env: {} key(s) configured", cfg.env.len()));
        }
    }
    Some(lines)
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

pub fn environment_block(working_dir: &Path, kind: AgentKind) -> String {
    let platform = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let date = chrono::Utc::now().format("%a %b %d %Y").to_string();
    let mut s = String::new();
    s.push_str("# Environment\n");
    s.push_str(&format!(
        "- Working directory: {} (may enter subdirectories, do not go outside it)\n",
        working_dir.display()
    ));
    s.push_str(&format!("- Platform: {platform}-{arch}\n"));
    s.push_str(&format!("- Date: {date}\n"));
    s.push_str("- You have file system and shell access via your tools. Run tools in parallel when independent.\n");
    // In PLAN mode the environment block carries a read-only marker so the
    // model is discouraged from attempting edits/writes (mutating bash is
    // intercepted anyway). Omitted in ACT mode to save tokens.
    if kind == AgentKind::Plan {
        s.push_str("- IN_PLAN_MODE: read-only — do not edit/write files; mutating bash is intercepted. Investigate read-only and output a plan only.\n");
    }
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

#[cfg(test)]
mod tests {
    use super::mcp_section;
    use opencoder_core::config::McpServerConfig;

    #[test]
    fn mcp_section_empty_returns_none() {
        assert!(mcp_section(&[]).is_none());
    }

    #[test]
    fn mcp_section_includes_enabled_server() {
        // enabled_mcp_servers already filters; but verify the helper works with
        // whatever it's given (it trusts the caller filtered).
        let cfg = McpServerConfig {
            enabled: true,
            command: Some("npx".to_string()),
            args: vec!["-y".to_string(), "@mcp/server".to_string()],
            ..Default::default()
        };
        let servers = vec![("active".to_string(), &cfg)];
        let s = mcp_section(&servers).unwrap();
        assert!(s.contains("## MCP Servers"));
        assert!(s.contains("active"));
        assert!(s.contains("stdio"));
        assert!(s.contains("npx"));
        assert!(s.contains("@mcp/server"));
    }

    #[test]
    fn mcp_section_sse_transport() {
        let cfg = McpServerConfig {
            enabled: true,
            url: Some("https://example.com/sse".to_string()),
            ..Default::default()
        };
        let servers = vec![("remote".to_string(), &cfg)];
        let s = mcp_section(&servers).unwrap();
        assert!(s.contains("(sse)"));
        assert!(s.contains("https://example.com/sse"));
    }

    #[test]
    fn mcp_section_no_transport() {
        let cfg = McpServerConfig {
            enabled: true,
            ..Default::default()
        };
        let servers = vec![("bare".to_string(), &cfg)];
        let s = mcp_section(&servers).unwrap();
        assert!(s.contains("no transport configured"));
    }
}
