//! `apply_skill_tokens_with` tests — pure app_helpers tests, extracted here
//! from `app_tests` (they test `app_helpers::apply_skill_tokens_with`, not
//! `app::handle_key`). Includes combined-content cases where a `$skill`
//! token is mixed with other input text.

// ---- apply_skill_tokens tests ----
// These tests resolve skills against a tempdir by passing
// `discover_in(tempdir)` straight into `apply_skill_tokens_with`, so they never
// touch the process-global `HOME` env var and need no serialization.

/// Create a tempdir whose `~/.opencoder/skills/<name>.md` contains a skill
/// with the given body, returning the tempdir (keep alive for the test).
fn skill_tempdir(name: &str, body: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let skills = dir.path().join(".opencoder").join("skills");
    std::fs::create_dir_all(&skills).unwrap();
    std::fs::write(skills.join(format!("{name}.md")), body).unwrap();
    dir
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn apply_skill_tokens_resolves_and_activates_known_skill() {
    let dir = skill_tempdir("alpha", "the alpha body");
    let skill_handle: std::sync::Arc<std::sync::Mutex<Option<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let mut active_skill = None;
    let mut active_skill_body = None;
    let mut sys_tokens = 0u64;
    let workdir = std::path::PathBuf::from("/tmp");

    let skills = opencoder_core::discover_in(&dir.path().join(".opencoder").join("skills"));
    let (clean, unresolved) = crate::app_helpers::apply_skill_tokens_with(
        &skills,
        "hello $alpha world",
        &mut active_skill,
        &mut active_skill_body,
        &mut sys_tokens,
        "act",
        &workdir,
        &skill_handle,
    );

    // Token stripped from clean text; name not unresolved.
    assert_eq!(clean, "hello  world");
    assert!(unresolved.is_empty(), "known skill must not be unresolved");
    // Skill activated (sticky display + body).
    assert_eq!(active_skill.as_deref(), Some("alpha"));
    let body = active_skill_body.as_deref().expect("skill body set");
    assert!(body.starts_with("> Source: "), "must prefix source path: {body}");
    assert!(body.ends_with("the alpha body"), "body must follow annotation: {body}");
    assert!(
        sys_tokens > 0,
        "sys_tokens must be recomputed with the skill body"
    );
    // The shared skill_handle (session.skill_prompt) is updated in-place.
    let handle_body = skill_handle.lock().unwrap();
    let handle_body = handle_body.as_deref().expect("skill_handle body set");
    assert!(handle_body.starts_with("> Source: "), "must prefix source path: {handle_body}");
    assert!(handle_body.ends_with("the alpha body"), "skill_handle must hold the resolved body: {handle_body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn apply_skill_tokens_reports_unknown_skill() {
    let dir = skill_tempdir("alpha", "alpha body");
    let skill_handle: std::sync::Arc<std::sync::Mutex<Option<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let mut active_skill = None;
    let mut active_skill_body = None;
    let mut sys_tokens = 0u64;
    let workdir = std::path::PathBuf::from("/tmp");

    let skills = opencoder_core::discover_in(&dir.path().join(".opencoder").join("skills"));
    let (clean, unresolved) = crate::app_helpers::apply_skill_tokens_with(
        &skills,
        "go $ghost now",
        &mut active_skill,
        &mut active_skill_body,
        &mut sys_tokens,
        "act",
        &workdir,
        &skill_handle,
    );

    // Unresolved `$ghost` is preserved verbatim (no content loss).
    assert_eq!(clean, "go $ghost now");
    assert_eq!(unresolved, vec!["ghost".to_string()]);
    // No skill resolved -> active skill untouched, sys_tokens unchanged.
    assert!(active_skill.is_none());
    assert!(active_skill_body.is_none());
    assert_eq!(
        sys_tokens, 0,
        "sys_tokens must not change when nothing resolves"
    );
    assert!(
        skill_handle.lock().unwrap().is_none(),
        "skill_handle must not be written when nothing resolves"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn apply_skill_tokens_no_tokens_leaves_skill_untouched() {
    let dir = skill_tempdir("alpha", "alpha body");
    let skill_handle: std::sync::Arc<std::sync::Mutex<Option<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Some("prior body".to_string())));
    let mut active_skill = Some("prior".to_string());
    let mut active_skill_body = Some("prior body".to_string());
    let mut sys_tokens = 999u64;
    let workdir = std::path::PathBuf::from("/tmp");

    let skills = opencoder_core::discover_in(&dir.path().join(".opencoder").join("skills"));
    let (clean, unresolved) = crate::app_helpers::apply_skill_tokens_with(
        &skills,
        "plain text no tokens",
        &mut active_skill,
        &mut active_skill_body,
        &mut sys_tokens,
        "act",
        &workdir,
        &skill_handle,
    );

    // No tokens -> text unchanged, nothing unresolved, sticky skill preserved.
    assert_eq!(clean, "plain text no tokens");
    assert!(unresolved.is_empty());
    assert_eq!(active_skill.as_deref(), Some("prior"));
    assert_eq!(active_skill_body.as_deref(), Some("prior body"));
    assert_eq!(sys_tokens, 999, "sys_tokens must not be recomputed");
    assert_eq!(
        skill_handle.lock().unwrap().as_deref(),
        Some("prior body"),
        "skill_handle must be untouched when no tokens present"
    );
}

// -----------------------------------------------------------------------
// Combined-content apply_skill_tokens_with: skill token mixed with other
// input text. These guard the exact scenario "submit a skill plus other
// content" — the skill must resolve AND the remaining text must survive.
// -----------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn apply_skill_tokens_combined_content_token_at_end() {
    // "do stuff $alpha" — token is at the tail, prose leads.
    let dir = skill_tempdir("alpha", "alpha body");
    let skill_handle: std::sync::Arc<std::sync::Mutex<Option<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let mut active_skill = None;
    let mut active_skill_body = None;
    let mut sys_tokens = 0u64;
    let workdir = std::path::PathBuf::from("/tmp");

    let skills = opencoder_core::discover_in(&dir.path().join(".opencoder").join("skills"));
    let (clean, unresolved) = crate::app_helpers::apply_skill_tokens_with(
        &skills,
        "do stuff $alpha",
        &mut active_skill,
        &mut active_skill_body,
        &mut sys_tokens,
        "act",
        &workdir,
        &skill_handle,
    );

    assert_eq!(clean, "do stuff ", "prose must survive token extraction");
    assert!(unresolved.is_empty());
    assert_eq!(
        active_skill.as_deref(),
        Some("alpha"),
        "skill must activate"
    );
    let body = active_skill_body.as_deref().expect("skill body set");
    assert!(body.starts_with("> Source: "), "must prefix source path: {body}");
    assert!(body.ends_with("alpha body"), "body must follow annotation: {body}");
    let handle_body = skill_handle.lock().unwrap();
    let handle_body = handle_body.as_deref().expect("skill_handle body set");
    assert!(handle_body.starts_with("> Source: "), "must prefix source path: {handle_body}");
    assert!(handle_body.ends_with("alpha body"), "skill_handle must carry the resolved body: {handle_body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn apply_skill_tokens_combined_content_multiple_skills_with_text() {
    // "$alpha do $beta stuff" — two skills interleaved with prose.
    let dir = skill_tempdir("alpha", "alpha body");
    // second skill in the same skills dir
    std::fs::write(
        dir.path().join(".opencoder").join("skills").join("beta.md"),
        "beta body",
    )
    .unwrap();

    let skill_handle: std::sync::Arc<std::sync::Mutex<Option<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let mut active_skill = None;
    let mut active_skill_body = None;
    let mut sys_tokens = 0u64;
    let workdir = std::path::PathBuf::from("/tmp");

    let skills = opencoder_core::discover_in(&dir.path().join(".opencoder").join("skills"));
    let (clean, unresolved) = crate::app_helpers::apply_skill_tokens_with(
        &skills,
        "$alpha do $beta stuff",
        &mut active_skill,
        &mut active_skill_body,
        &mut sys_tokens,
        "act",
        &workdir,
        &skill_handle,
    );

    assert_eq!(clean, " do  stuff");
    assert!(unresolved.is_empty());
    // Both skill names appear in the sticky display, both bodies joined.
    assert_eq!(active_skill.as_deref(), Some("alpha, beta"));
    let body = active_skill_body.as_deref().expect("skill body set");
    assert!(body.starts_with("> Source: "), "must prefix source path: {body}");
    assert!(
        body.contains("alpha body") && body.contains("beta body"),
        "both bodies present in first-seen order: {body}"
    );
    assert!(body.ends_with("beta body"), "last joined body must be beta: {body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn apply_skill_tokens_combined_mixed_resolved_and_unresolved() {
    // "$alpha do $ghost stuff" — one known, one unknown.
    let dir = skill_tempdir("alpha", "alpha body");
    let skill_handle: std::sync::Arc<std::sync::Mutex<Option<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let mut active_skill = None;
    let mut active_skill_body = None;
    let mut sys_tokens = 0u64;
    let workdir = std::path::PathBuf::from("/tmp");

    let skills = opencoder_core::discover_in(&dir.path().join(".opencoder").join("skills"));
    let (clean, unresolved) = crate::app_helpers::apply_skill_tokens_with(
        &skills,
        "$alpha do $ghost stuff",
        &mut active_skill,
        &mut active_skill_body,
        &mut sys_tokens,
        "act",
        &workdir,
        &skill_handle,
    );

    // Resolved `alpha` is stripped; unresolved `$ghost` preserved verbatim.
    assert_eq!(clean, " do $ghost stuff");
    // Only the known skill resolves; the unknown is reported back.
    assert_eq!(unresolved, vec!["ghost"]);
    assert_eq!(active_skill.as_deref(), Some("alpha"));
    let handle_body = skill_handle.lock().unwrap();
    let handle_body = handle_body.as_deref().expect("skill_handle body set");
    assert!(handle_body.starts_with("> Source: "), "must prefix source path: {handle_body}");
    assert!(
        handle_body.ends_with("alpha body"),
        "known body must follow annotation despite unknown peer: {handle_body}"
    );
}
