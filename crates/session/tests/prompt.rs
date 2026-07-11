//! Prompt construction tests — verifies build_system, environment_block,
//! and compaction prompts produce correct content.

use opencode_core::resolve_agent;
use opencode_session::prompt::{
    build_system, compaction_system_prompt, compaction_user_prompt, environment_block,
};

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
