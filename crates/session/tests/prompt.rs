//! Prompt construction tests — verifies build_system, environment_block,
//! and compaction prompts produce correct content.

use opencoder_core::{resolve_agent, AgentKind};
use opencoder_session::prompt::{
    build_system, compaction_system_prompt, compaction_user_prompt, environment_block,
};
use std::sync::Mutex;

#[test]
fn build_system_includes_agent_prompt_and_environment() {
    let agent = resolve_agent("act").unwrap();
    let dir = std::path::Path::new("/tmp/project");
    let msg = build_system(&agent, dir, None, None);
    let text = msg.text();
    // Agent base prompt is included
    assert!(!text.is_empty());
    // Environment block is appended
    assert!(text.contains("Working directory"));
    assert!(text.contains("/tmp/project"));
}

#[test]
fn build_system_contains_no_skill_section() {
    let agent = resolve_agent("act").unwrap();
    let dir = std::path::Path::new("/tmp/project");
    let msg = build_system(&agent, dir, None, None);
    // Skill bodies never ship in the system prompt (they moved to a
    // transient tail reminder; see `skill_context`), so skill activation
    // never rewrites the payload's first bytes.
    assert!(!msg.text().contains("Active skill"));
}

#[test]
fn task_plan_act_system_prompt_strips_build_delegation() {
    let agent = resolve_agent("act").unwrap();
    let dir = std::path::Path::new("/tmp/project");
    let plan = "> Source: /home/u/.opencoder/skills/task-plan/SKILL.md\n\nplan body";

    // While task-plan is active the prompt must not advertise the 'build'
    // subagent; the 'explore' advertisement survives.
    let msg = build_system(&agent, dir, None, Some(plan));
    assert!(
        !msg.text().contains("'build' (full tools)"),
        "task-plan act prompt must not advertise build, got: {}",
        msg.text()
    );
    assert!(msg.text().contains("'explore' (read-only)"));

    // Any other skill (or no skill) keeps the full delegation line.
    let review = "> Source: /home/u/.opencoder/skills/review/SKILL.md\n\nbody";
    assert!(build_system(&agent, dir, None, Some(review))
        .text()
        .contains("'build' (full tools)"));
    assert!(build_system(&agent, dir, None, None)
        .text()
        .contains("'build' (full tools)"));

    // The sandbox agent prompt is pre-stripped; task-plan stripping is a
    // no-op there, so the read-only contract is never weakened.
    let sandbox = resolve_agent("sandbox").unwrap();
    assert!(!build_system(&sandbox, dir, None, None)
        .text()
        .contains("'build' (full tools)"));
}

#[test]
fn environment_block_contains_cwd_and_platform() {
    let block = environment_block(std::path::Path::new("/home/user/repo"), AgentKind::Act);
    assert!(block.contains("Working directory: /home/user/repo"));
    assert!(block.contains("Platform:"));
    assert!(block.contains("Date:"));
}

#[test]
fn compaction_system_prompt_is_anchored_summarizer() {
    let p = compaction_system_prompt();
    assert!(p.to_lowercase().contains("summar"));
    assert!(p.contains("anchored"));
    assert!(p.contains("<previous-summary>"));
}

#[test]
fn compaction_user_prompt_has_all_structured_sections() {
    let p = compaction_user_prompt(None);
    assert!(p.contains("## Objective"));
    assert!(p.contains("## Important Details"));
    assert!(p.contains("## Work State"));
    assert!(p.contains("### Completed"));
    assert!(p.contains("### Active"));
    assert!(p.contains("### Blocked"));
    assert!(p.contains("## Next Move"));
    assert!(p.contains("## Relevant Files"));
    assert!(p.contains("<template>"));
}

#[test]
fn compaction_user_prompt_includes_previous_summary_when_provided() {
    let p = compaction_user_prompt(Some("## Objective\n- Do the thing"));
    assert!(p.contains("<previous-summary>"));
    assert!(p.contains("Do the thing"));
    assert!(p.contains("Update the anchored summary"));
}

#[test]
fn compaction_user_prompt_without_previous_summary_says_create_new() {
    let p = compaction_user_prompt(None);
    assert!(p.contains("Create a new anchored summary"));
    assert!(!p.contains("<previous-summary>"));
}

#[test]
fn environment_block_constrains_to_working_directory() {
    let block = environment_block(std::path::Path::new("/home/user/repo"), AgentKind::Act);
    assert!(block.contains("may enter subdirectories"));
    assert!(block.contains("do not go outside it"));
}

// ---------------------------------------------------------------------------
// AGENTS.md auto-loading tests
// ---------------------------------------------------------------------------

/// Serialize tests that touch the `HOME` environment variable so they don't
/// interfere with each other or with the rest of the test suite.
static HOME_MUTEX: Mutex<()> = Mutex::new(());

fn with_home<R>(home: &std::path::Path, f: impl FnOnce() -> R) -> R {
    let _guard = HOME_MUTEX.lock().unwrap();
    let old = std::env::var_os("HOME");
    std::env::set_var("HOME", home);
    let result = f();
    match old {
        Some(h) => std::env::set_var("HOME", h),
        None => std::env::remove_var("HOME"),
    }
    result
}

#[test]
fn project_instructions_from_working_dir_only() {
    let home = tempfile::TempDir::new().unwrap();
    let working = tempfile::TempDir::new().unwrap();
    std::fs::write(working.path().join("AGENTS.md"), "Use Rust 2021 edition.").unwrap();

    with_home(home.path(), || {
        let agent = resolve_agent("act").unwrap();
        let msg = build_system(&agent, working.path(), None, None);
        let text = msg.text();
        assert!(text.contains("## Project instructions"));
        assert!(text.contains("Use Rust 2021 edition."));
    });
}

#[test]
fn project_instructions_from_global_and_working_dir() {
    let home = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(home.path().join(".opencoder")).unwrap();
    std::fs::write(
        home.path().join(".opencoder").join("AGENTS.md"),
        "Global rule.",
    )
    .unwrap();

    let working = tempfile::TempDir::new().unwrap();
    std::fs::write(working.path().join("AGENTS.md"), "Local rule.").unwrap();

    with_home(home.path(), || {
        let agent = resolve_agent("act").unwrap();
        let msg = build_system(&agent, working.path(), None, None);
        let text = msg.text();
        assert!(text.contains("## Project instructions"));
        assert!(text.contains("Global rule."));
        assert!(text.contains("Local rule."));
        // Global comes before local (lower priority first)
        let g = text.find("Global rule.").unwrap();
        let l = text.find("Local rule.").unwrap();
        assert!(g < l);
    });
}

#[test]
fn project_instructions_from_git_root_when_in_subdir() {
    let home = tempfile::TempDir::new().unwrap();

    let repo = tempfile::TempDir::new().unwrap();
    std::fs::create_dir(repo.path().join(".git")).unwrap();
    std::fs::write(repo.path().join("AGENTS.md"), "Repo-wide rule.").unwrap();

    let subdir = repo.path().join("src").join("deep");
    std::fs::create_dir_all(&subdir).unwrap();

    with_home(home.path(), || {
        let agent = resolve_agent("act").unwrap();
        let msg = build_system(&agent, &subdir, None, None);
        let text = msg.text();
        assert!(text.contains("## Project instructions"));
        assert!(text.contains("Repo-wide rule."));
    });
}

#[test]
fn project_instructions_absent_when_no_agents_md() {
    let home = tempfile::TempDir::new().unwrap();
    let working = tempfile::TempDir::new().unwrap();

    with_home(home.path(), || {
        let agent = resolve_agent("act").unwrap();
        let msg = build_system(&agent, working.path(), None, None);
        let text = msg.text();
        assert!(!text.contains("## Project instructions"));
    });
}

#[test]
fn project_instructions_case_insensitive_lowercase() {
    let home = tempfile::TempDir::new().unwrap();
    let working = tempfile::TempDir::new().unwrap();
    std::fs::write(working.path().join("agents.md"), "Lowercase filename.").unwrap();

    with_home(home.path(), || {
        let agent = resolve_agent("act").unwrap();
        let msg = build_system(&agent, working.path(), None, None);
        let text = msg.text();
        assert!(text.contains("## Project instructions"));
        assert!(text.contains("Lowercase filename."));
    });
}

#[test]
fn project_instructions_case_insensitive_uppercase_ext() {
    let home = tempfile::TempDir::new().unwrap();
    let working = tempfile::TempDir::new().unwrap();
    std::fs::write(working.path().join("AGENTS.MD"), "Uppercase ext.").unwrap();

    with_home(home.path(), || {
        let agent = resolve_agent("act").unwrap();
        let msg = build_system(&agent, working.path(), None, None);
        let text = msg.text();
        assert!(text.contains("## Project instructions"));
        assert!(text.contains("Uppercase ext."));
    });
}

#[test]
fn project_instructions_prefers_exact_agents_md_name_over_variants() {
    let home = tempfile::TempDir::new().unwrap();
    let working = tempfile::TempDir::new().unwrap();
    std::fs::write(working.path().join("agents.md"), "Lowercase variant body.").unwrap();
    std::fs::write(working.path().join("AGENTS.MD"), "Upper-ext variant body.").unwrap();
    std::fs::write(working.path().join("AGENTS.md"), "Exact-name body wins.").unwrap();

    with_home(home.path(), || {
        let agent = resolve_agent("act").unwrap();
        let msg = build_system(&agent, working.path(), None, None);
        let text = msg.text();
        assert!(text.contains("## Project instructions"));
        // Exactly one file is loaded per directory: the exact `AGENTS.md`
        // name wins over case-insensitive variants, regardless of
        // read_dir order.
        assert!(text.contains("Exact-name body wins."));
        assert!(!text.contains("Lowercase variant body."));
        assert!(!text.contains("Upper-ext variant body."));
        assert_eq!(text.matches("## Project instructions").count(), 1);
    });
}

#[test]
fn project_instructions_without_exact_name_picks_smallest_variant() {
    let home = tempfile::TempDir::new().unwrap();
    let working = tempfile::TempDir::new().unwrap();
    // No exact `AGENTS.md`: the lexicographically smallest matching file
    // name is chosen (OsString byte order: 'M' (0x4D) < 'm' (0x6D), so
    // `AGENTS.MD` sorts before `agents.md`).
    std::fs::write(working.path().join("agents.md"), "Lowercase variant body.").unwrap();
    std::fs::write(working.path().join("AGENTS.MD"), "Upper-ext variant body.").unwrap();

    with_home(home.path(), || {
        let agent = resolve_agent("act").unwrap();
        let msg = build_system(&agent, working.path(), None, None);
        let text = msg.text();
        assert!(text.contains("## Project instructions"));
        assert!(text.contains("Upper-ext variant body."));
        assert!(!text.contains("Lowercase variant body."));
    });
}

#[test]
fn project_instructions_dedup_when_git_root_is_working_dir() {
    let home = tempfile::TempDir::new().unwrap();

    let repo = tempfile::TempDir::new().unwrap();
    std::fs::create_dir(repo.path().join(".git")).unwrap();
    std::fs::write(repo.path().join("AGENTS.md"), "Single rule.").unwrap();

    with_home(home.path(), || {
        let agent = resolve_agent("act").unwrap();
        let msg = build_system(&agent, repo.path(), None, None);
        let text = msg.text();
        assert!(text.contains("## Project instructions"));
        // The content must appear exactly once (dedup: git root == working dir)
        let count = text.matches("Single rule.").count();
        assert_eq!(count, 1);
    });
}

#[test]
fn project_instructions_appears_before_environment() {
    let home = tempfile::TempDir::new().unwrap();
    let working = tempfile::TempDir::new().unwrap();
    std::fs::write(working.path().join("AGENTS.md"), "My rule.").unwrap();

    with_home(home.path(), || {
        let agent = resolve_agent("act").unwrap();
        let msg = build_system(&agent, working.path(), None, None);
        let text = msg.text();
        let instr_pos = text.find("## Project instructions").unwrap();
        let env_pos = text.find("# Environment").unwrap();
        assert!(instr_pos < env_pos);
    });
}

#[test]
fn project_instructions_small_file_not_truncated() {
    let home = tempfile::TempDir::new().unwrap();
    let working = tempfile::TempDir::new().unwrap();
    std::fs::write(working.path().join("AGENTS.md"), "Small rule file.").unwrap();

    with_home(home.path(), || {
        let agent = resolve_agent("act").unwrap();
        let msg = build_system(&agent, working.path(), None, None);
        let text = msg.text();
        assert!(text.contains("Small rule file."));
        assert!(!text.contains("[AGENTS.md truncated"));
    });
}

#[test]
fn project_instructions_truncated_past_200kb_with_boundary_safe_cut() {
    let cap: usize = 200 * 1024;
    let home = tempfile::TempDir::new().unwrap();
    let working = tempfile::TempDir::new().unwrap();

    // Single-byte prefix ending 2 bytes before the cap, then 4-byte emoji
    // that STRADDLE the cap boundary (forcing a char-boundary walk-back),
    // then distinctive trailing content that must never leak into the prompt.
    let mut content = "Q".repeat(cap - 2);
    content.push_str(&"🎉".repeat(64));
    content.push_str("TRAILING_BEYOND_CAP_MARKER");
    assert!(content.len() > cap);
    std::fs::write(working.path().join("AGENTS.md"), &content).unwrap();

    with_home(home.path(), || {
        let agent = resolve_agent("act").unwrap();
        let msg = build_system(&agent, working.path(), None, None);
        let text = msg.text();

        let marker = format!(
            "[AGENTS.md truncated: original size {} bytes exceeds 200KB limit]",
            content.len()
        );
        assert!(text.contains(&marker));

        // Extract exactly the instructions section from the prompt.
        let header = "## Project instructions\n";
        let start = text.find(header).unwrap() + header.len();
        let end = text.find("\n\n# Environment").unwrap();
        let section = &text[start..end];

        // The head is the cap-sized prefix cut at a char boundary: the emoji
        // straddling byte `cap` is dropped, leaving only the Q-run.
        let head_len = cap - 2;
        assert_eq!(&section[..head_len], "Q".repeat(head_len));
        assert_eq!(section.len(), head_len + "\n\n".len() + marker.len());
        // Nothing beyond the cap (emoji run, trailing marker) leaks through.
        assert!(!section.contains('🎉'));
        assert!(!text.contains('🎉'));
        assert!(!text.contains("TRAILING_BEYOND_CAP_MARKER"));
    });
}

#[test]
fn environment_block_marks_sandbox_mode_readonly() {
    let block = environment_block(std::path::Path::new("/repo"), AgentKind::Sandbox);
    assert!(block.contains("IN_SANDBOX_MODE"));
    assert!(block.contains("read-only"));
    // Updated for the shellguard release set: the marker now names what IS
    // permitted (/tmp writes, /dev/null redirects) and states that the
    // project/working directory is NOT writable, instead of the blanket
    // "do not edit/write files".
    assert!(block.contains("writes under /tmp"));
    assert!(block.contains("redirects to /dev/null"));
    assert!(block.contains("NOT writable"));
}

#[test]
fn environment_block_omits_sandbox_marker_in_act() {
    let block = environment_block(std::path::Path::new("/repo"), AgentKind::Act);
    assert!(!block.contains("IN_SANDBOX_MODE"));
    assert!(block.contains("Working directory: /repo"));
}
