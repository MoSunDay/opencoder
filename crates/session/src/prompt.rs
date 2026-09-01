use crate::mcp::ConnStatus;
use opencoder_core::{message::now_ms, AgentKind, CliConfig, Message};
use std::path::{Path, PathBuf};

pub fn build_system(
    agent: &opencoder_core::Agent,
    working_dir: &Path,
    mcp_block: Option<&str>,
    skill_body: Option<&str>,
) -> Message {
    // While the task-plan skill is active the prompt must not advertise the
    // 'build' (implementation) subagent: plan-only turns are not driven
    // toward implementation delegation. Same mechanism as sandbox mode,
    // whose agent prompt is pre-stripped (making this replace a no-op
    // there). Every other prompt passes through unchanged.
    let mut text = agent.prompt.clone();
    if crate::tools::latent::task_plan_active(skill_body) {
        text = opencoder_core::strip_build_delegation(&text);
    }

    if let Some(instructions) = load_instructions(working_dir) {
        text.push_str("\n\n## Project instructions\n");
        text.push_str(&instructions);
    }

    let env = environment_block(working_dir, agent.kind);
    text.push_str("\n\n");
    text.push_str(&env);

    if let Some(mcp) = mcp_block {
        let trimmed = mcp.trim();
        if !trimmed.is_empty() {
            text.push_str("\n\n");
            text.push_str(trimmed);
        }
    }

    Message::system("system", text)
}

/// Build the `## MCP Servers` system-prompt section from live pool status.
///
/// Returns `None` when there are no MCP connections (zero behaviour change
/// for sessions without MCP). When servers are connected, lists each server
/// name and tool count. Failed connections are surfaced so the model is
/// aware of unavailable tools.
pub fn mcp_section(status: &[(String, ConnStatus)]) -> Option<String> {
    if status.is_empty() {
        return None;
    }
    let mut lines = String::from("## MCP Servers\n");
    lines.push_str(
        "The following MCP (Model Context Protocol) servers are connected. \
         Their tools are available as regular tools, prefixed with `mcp__`. \
         Call them like any other tool.",
    );
    for (name, st) in status {
        lines.push_str("\n\n### ");
        lines.push_str(name);
        match st {
            ConnStatus::Connected { tool_count } => {
                lines.push_str(&format!(" — connected, {tool_count} tool(s) available"));
            }
            ConnStatus::Failed(msg) => {
                lines.push_str(" — connection failed: ");
                lines.push_str(msg);
            }
        }
    }
    Some(lines)
}

/// Build the system-prompt section for enabled CLI registrations.
pub fn cli_section(entries: &[(String, &CliConfig)]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    let mut text = String::from(
        "## Registered CLI\nThe following user-registered command-line interfaces are enabled. Follow each registration's usage contract when using that CLI.",
    );
    for (name, cfg) in entries {
        let content = cfg.content.trim();
        if content.is_empty() {
            continue;
        }
        text.push_str("\n\n### ");
        text.push_str(name);
        text.push('\n');
        text.push_str(content);
    }
    Some(text)
}

/// Join optional runtime prompt sections while preserving zero-config behavior.
pub fn runtime_sections(mcp: Option<&str>, cli: Option<&str>) -> Option<String> {
    let sections: Vec<&str> = [mcp, cli]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|section| !section.is_empty())
        .collect();
    (!sections.is_empty()).then(|| sections.join("\n\n"))
}

/// Maximum number of bytes of a single AGENTS.md file included in the
/// system prompt. Files whose (trimmed) content exceeds this are truncated
/// to the first `AGENTS_MD_MAX_BYTES` bytes plus a marker line carrying the
/// original size.
pub(crate) const AGENTS_MD_MAX_BYTES: usize = 200 * 1024;

/// Truncate `s` to at most `max` bytes without splitting a UTF-8 character:
/// slice on the byte limit, then walk back to the nearest char boundary.
fn truncate_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Apply the AGENTS_MD_MAX_BYTES cap to one trimmed instruction body.
/// Within the limit the body passes through unchanged; past it, keep the
/// first `AGENTS_MD_MAX_BYTES` bytes (char-boundary safe) and append a
/// marker noting the original size.
fn cap_instructions(trimmed: &str) -> String {
    if trimmed.len() <= AGENTS_MD_MAX_BYTES {
        return trimmed.to_string();
    }
    let head = truncate_bytes(trimmed, AGENTS_MD_MAX_BYTES);
    format!(
        "{head}\n\n[AGENTS.md truncated: original size {} bytes exceeds {}KB limit]",
        trimmed.len(),
        AGENTS_MD_MAX_BYTES / 1024
    )
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
                    parts.push(cap_instructions(trimmed));
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

/// Find the AGENTS.md file inside `dir`, matching the file name
/// case-insensitively.
///
/// Deterministic regardless of `read_dir` order: collect every matching
/// regular file first, then pick (1) an entry named exactly `AGENTS.md` if
/// present, otherwise (2) the lexicographically smallest matching file name
/// (`OsString` byte order; e.g. `AGENTS.MD` < `agents.md` because `b'M'` <
/// `b'm'` in ASCII). Never returns based on directory iteration order.
fn find_agents_md(dir: &Path) -> Option<PathBuf> {
    let mut names: Vec<std::ffi::OsString> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|entry| entry.file_name().eq_ignore_ascii_case("AGENTS.md"))
        .filter(|entry| entry.path().is_file())
        .map(|entry| entry.file_name())
        .collect();
    // read_dir order is filesystem-dependent; sort to impose our own rule.
    names.sort();
    let chosen = names
        .iter()
        .find(|name| name.as_os_str() == std::ffi::OsStr::new("AGENTS.md"))
        .or_else(|| names.first())?;
    Some(dir.join(chosen))
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
    // Plan alone gets an explicit mode row. ACT intentionally omits the row:
    // execution is the default and needs no extra mode description.
    if kind == AgentKind::Plan {
        s.push_str("- MODE: plan (read-only); IN_PLAN_MODE=true — do not edit or write files and do not execute implementation. Every state-changing operation is intercepted and returned in your context. If blocked, do not retry or find another write path; continue read-only analysis and output a focused plan only.\n");
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
    use super::{cap_instructions, mcp_section, truncate_bytes, AGENTS_MD_MAX_BYTES};
    use crate::mcp::ConnStatus;

    #[test]
    fn mcp_section_empty_returns_none() {
        assert!(mcp_section(&[]).is_none());
    }

    #[test]
    fn mcp_section_connected_shows_tool_count() {
        let status = vec![(
            "active".to_string(),
            ConnStatus::Connected { tool_count: 3 },
        )];
        let s = mcp_section(&status).unwrap();
        assert!(s.contains("## MCP Servers"));
        assert!(s.contains("active"));
        assert!(s.contains("3 tool"));
        assert!(s.contains("mcp__"));
    }

    #[test]
    fn mcp_section_failed_shows_error_message() {
        let status = vec![(
            "broken".to_string(),
            ConnStatus::Failed("spawn failed: ENOENT".into()),
        )];
        let s = mcp_section(&status).unwrap();
        assert!(s.contains("broken"));
        assert!(s.contains("connection failed"));
        assert!(s.contains("ENOENT"));
    }

    #[test]
    fn mcp_section_mixed_statuses() {
        let status = vec![
            ("ok".to_string(), ConnStatus::Connected { tool_count: 2 }),
            ("bad".to_string(), ConnStatus::Failed("timeout".into())),
        ];
        let s = mcp_section(&status).unwrap();
        assert!(s.contains("ok"));
        assert!(s.contains("2 tool"));
        assert!(s.contains("bad"));
        assert!(s.contains("timeout"));
    }

    #[test]
    fn truncate_bytes_returns_input_when_within_limit() {
        assert_eq!(truncate_bytes("hello", 10), "hello");
        assert_eq!(truncate_bytes("hello", 5), "hello");
        assert_eq!(truncate_bytes("", 0), "");
    }

    #[test]
    fn truncate_bytes_cuts_on_char_boundary() {
        // 'é' is 2 bytes: cutting inside it must walk back to the boundary.
        let s = "abcéxyz"; // bytes: a b c [c3 a9] x y z  (len 7)
        assert_eq!(truncate_bytes(s, 4), "abc"); // byte 4 is mid-'é'
        assert_eq!(truncate_bytes(s, 5), "abcé");
        assert_eq!(truncate_bytes(s, 6), "abcéx");
    }

    #[test]
    fn cap_instructions_under_limit_passes_through() {
        let body = "short body";
        assert_eq!(cap_instructions(body), body);
    }

    #[test]
    fn cap_instructions_over_limit_truncates_with_marker() {
        let body = "x".repeat(AGENTS_MD_MAX_BYTES + 10);
        let capped = cap_instructions(&body);
        assert!(capped.starts_with(&"x".repeat(AGENTS_MD_MAX_BYTES)));
        assert_eq!(
            capped
                .strip_prefix(&"x".repeat(AGENTS_MD_MAX_BYTES))
                .unwrap(),
            format!(
                "\n\n[AGENTS.md truncated: original size {} bytes exceeds {}KB limit]",
                body.len(),
                AGENTS_MD_MAX_BYTES / 1024
            )
        );
    }
}
