//! Prompt construction tests — verifies build_system, environment_block,
//! and compaction_prompt produce correct content.

use opencode_core::resolve_agent;
use opencode_session::prompt::{build_system, compaction_prompt, environment_block};

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
fn compaction_prompt_is_non_empty_and_mentions_summary() {
    let p = compaction_prompt();
    assert!(!p.is_empty());
    assert!(p.to_lowercase().contains("summar"));
}
