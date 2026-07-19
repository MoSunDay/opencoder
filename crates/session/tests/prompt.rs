//! Prompt construction tests — verifies build_system, environment_block,
//! and compaction prompts produce correct content.

use opencoder_core::resolve_agent;
use opencoder_session::prompt::{
    build_system, compaction_system_prompt, compaction_user_prompt, environment_block,
    global_instructions_text,
};
use std::sync::Mutex;

#[test]
fn build_system_includes_agent_prompt_and_environment() {
    let agent = resolve_agent("act").unwrap();
    let dir = std::path::Path::new("/tmp/project");
    let msg = build_system(&agent, dir, None);
    let text = msg.text();
    // Agent base prompt is included
    assert!(!text.is_empty());
    // Environment block is appended
    assert!(text.contains("Working directory"));
    assert!(text.contains("/tmp/project"));
}

#[test]
fn build_system_appends_skill_when_provided() {
    let agent = resolve_agent("act").unwrap();
    let dir = std::path::Path::new("/tmp");
    let msg = build_system(&agent, dir, Some("Always use tabs for indentation."));
    let text = msg.text();
    assert!(text.contains("Active skill"));
    assert!(text.contains("Always use tabs"));
}

#[test]
fn build_system_omits_skill_section_when_empty() {
    let agent = resolve_agent("act").unwrap();
    let dir = std::path::Path::new("/tmp");
    let msg = build_system(&agent, dir, Some("   "));
    let text = msg.text();
    assert!(!text.contains("Active skill"));
}

#[test]
fn environment_block_contains_cwd_and_platform() {
    let block = environment_block(std::path::Path::new("/home/user/repo"));
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
    let block = environment_block(std::path::Path::new("/home/user/repo"));
    assert!(block.contains("Stay within the working directory"));
    assert!(block.contains("subdirectories"));
    assert!(block.contains("do not access or modify anything outside it"));
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
        let msg = build_system(&agent, working.path(), None);
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
        let msg = build_system(&agent, working.path(), None);
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
        let msg = build_system(&agent, &subdir, None);
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
        let msg = build_system(&agent, working.path(), None);
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
        let msg = build_system(&agent, working.path(), None);
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
        let msg = build_system(&agent, working.path(), None);
        let text = msg.text();
        assert!(text.contains("## Project instructions"));
        assert!(text.contains("Uppercase ext."));
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
        let msg = build_system(&agent, repo.path(), None);
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
        let msg = build_system(&agent, working.path(), None);
        let text = msg.text();
        let instr_pos = text.find("## Project instructions").unwrap();
        let env_pos = text.find("# Environment").unwrap();
        assert!(instr_pos < env_pos);
    });
}

// ---------------------------------------------------------------------------
// global_instructions_text tests (global agents.md excluded from ctx tokens)
// ---------------------------------------------------------------------------

#[test]
fn global_instructions_returns_global_agents_md_content() {
    let home = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(home.path().join(".opencoder")).unwrap();
    std::fs::write(
        home.path().join(".opencoder").join("AGENTS.md"),
        "Global baseline rule.",
    )
    .unwrap();
    let working = tempfile::TempDir::new().unwrap();

    with_home(home.path(), || {
        let got = global_instructions_text(working.path());
        assert_eq!(got.as_deref(), Some("Global baseline rule."));
    });
}

#[test]
fn global_instructions_none_when_no_global_file() {
    let home = tempfile::TempDir::new().unwrap();
    let working = tempfile::TempDir::new().unwrap();
    // A local working-dir agents.md must NOT be mistaken for the global one.
    std::fs::write(working.path().join("AGENTS.md"), "Local only.").unwrap();

    with_home(home.path(), || {
        assert_eq!(global_instructions_text(working.path()), None);
    });
}

#[test]
fn global_instructions_none_when_global_file_empty() {
    let home = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(home.path().join(".opencoder")).unwrap();
    std::fs::write(home.path().join(".opencoder").join("AGENTS.md"), "   \n\n  ").unwrap();
    let working = tempfile::TempDir::new().unwrap();

    with_home(home.path(), || {
        assert_eq!(global_instructions_text(working.path()), None);
    });
}

#[test]
fn global_instructions_ignores_git_root_and_working_dir_files() {
    // Only the home/.opencoder file is "global"; git-root and working-dir
    // agents.md files are local and must never be reported here.
    let home = tempfile::TempDir::new().unwrap();
    let working = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(working.path().join(".git")).unwrap();
    std::fs::write(working.path().join("AGENTS.md"), "Working rule.").unwrap();

    with_home(home.path(), || {
        assert_eq!(global_instructions_text(working.path()), None);
    });
}
