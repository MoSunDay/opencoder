//! Regression test for Bug 4: headless skill-token stripping must preserve
//! unresolved $name sequences as literal text instead of deleting them.

#[test]
fn unresolved_skill_token_preserved_in_prompt() {
    let prompt = "fix $nonexistent-skill and $real-skill please";
    let mut resolved = std::collections::HashSet::new();
    resolved.insert("real-skill".to_string());

    let cleaned = opencoder_core::strip_resolved_skill_tokens(prompt, &resolved);
    assert!(
        cleaned.contains("$nonexistent-skill"),
        "unresolved token must be preserved: {cleaned}"
    );
    assert!(
        !cleaned.contains("$real-skill"),
        "resolved token must be stripped: {cleaned}"
    );

    let (old_clean, _) = opencoder_core::extract_skill_tokens(prompt);
    assert!(
        !old_clean.contains("$nonexistent-skill"),
        "extract_skill_tokens strips ALL tokens (the bug being fixed)"
    );
}
