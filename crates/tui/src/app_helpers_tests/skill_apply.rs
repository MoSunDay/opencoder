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
    assert!(
        body.starts_with("> Source: "),
        "must prefix source path: {body}"
    );
    assert!(
        body.ends_with("the alpha body"),
        "body must follow annotation: {body}"
    );
    assert!(
        sys_tokens > 0,
        "sys_tokens must be recomputed with the skill body"
    );
    // The shared skill_handle (session.skill_prompt) is updated in-place.
    let handle_body = skill_handle.lock().unwrap();
    let handle_body = handle_body.as_deref().expect("skill_handle body set");
    assert!(
        handle_body.starts_with("> Source: "),
        "must prefix source path: {handle_body}"
    );
    assert!(
        handle_body.ends_with("the alpha body"),
        "skill_handle must hold the resolved body: {handle_body}"
    );
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
    assert!(
        body.starts_with("> Source: "),
        "must prefix source path: {body}"
    );
    assert!(
        body.ends_with("alpha body"),
        "body must follow annotation: {body}"
    );
    let handle_body = skill_handle.lock().unwrap();
    let handle_body = handle_body.as_deref().expect("skill_handle body set");
    assert!(
        handle_body.starts_with("> Source: "),
        "must prefix source path: {handle_body}"
    );
    assert!(
        handle_body.ends_with("alpha body"),
        "skill_handle must carry the resolved body: {handle_body}"
    );
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
    assert!(
        body.starts_with("> Source: "),
        "must prefix source path: {body}"
    );
    assert!(
        body.contains("alpha body") && body.contains("beta body"),
        "both bodies present in first-seen order: {body}"
    );
    assert!(
        body.ends_with("beta body"),
        "last joined body must be beta: {body}"
    );
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
    assert!(
        handle_body.starts_with("> Source: "),
        "must prefix source path: {handle_body}"
    );
    assert!(
        handle_body.ends_with("alpha body"),
        "known body must follow annotation despite unknown peer: {handle_body}"
    );
}

#[test]
fn refresh_skill_mirrors_syncs_name_body_and_tokens_from_handle() {
    use crate::app_helpers::refresh_skill_mirrors;
    use std::sync::{Arc, Mutex};

    let workdir = std::env::temp_dir();
    let skill_handle: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let (mut active_skill, mut active_skill_body, mut sys_tokens) = (None, None, 0u64);
    let mut plan_flag = false;

    // Stale local state (e.g. after a task switch or before any skill): the
    // runner activated a skill at consumption time — mirror it.
    let body = "> Source: /skills/haiku/SKILL.md\n\nAlways answer in haiku form.".to_string();
    *skill_handle.lock().unwrap() = Some(body.clone());
    refresh_skill_mirrors(
        &skill_handle,
        &mut active_skill,
        &mut active_skill_body,
        &mut sys_tokens,
        "act",
        &workdir,
        &mut plan_flag,
    );
    assert_eq!(
        active_skill.as_deref(),
        Some("haiku"),
        "name from Source prefix"
    );
    assert_eq!(active_skill_body.as_deref(), Some(body.as_str()));
    assert!(
        sys_tokens > 0,
        "sys_tokens re-estimated from the new skill body"
    );

    // No drift: an unchanged handle is a no-op (tokens not re-estimated).
    sys_tokens = 0;
    refresh_skill_mirrors(
        &skill_handle,
        &mut active_skill,
        &mut active_skill_body,
        &mut sys_tokens,
        "act",
        &workdir,
        &mut plan_flag,
    );
    assert_eq!(sys_tokens, 0, "no-op when handle matches the mirror");

    // Runner cleared the skill (plan handoff): mirrors clear too.
    *skill_handle.lock().unwrap() = None;
    refresh_skill_mirrors(
        &skill_handle,
        &mut active_skill,
        &mut active_skill_body,
        &mut sys_tokens,
        "act",
        &workdir,
        &mut plan_flag,
    );
    assert_eq!(active_skill, None);
    assert_eq!(active_skill_body, None);
}

/// The `[act]` task-plan chip flag is re-derived only when the body actually
/// changed: a committed `task-plan` body arms it, any other skill body (or a
/// cleared skill) disarms it, and an unchanged body (early return) must keep
/// the caller's value: the yellow a steer/queued input just cleared must not
/// be revived by a later idle mirror refresh.
#[test]
fn refresh_skill_mirrors_derives_plan_flag_only_on_body_change() {
    use crate::app_helpers::refresh_skill_mirrors;
    use std::sync::{Arc, Mutex};

    let workdir = std::env::temp_dir();
    let plan_body = "> Source: /skills/task-plan/SKILL.md\n\nplan".to_string();
    let review_body = "> Source: /skills/review/SKILL.md\n\nreview".to_string();
    let skill_handle: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let (mut active_skill, mut active_skill_body, mut sys_tokens) = (None, None, 0u64);

    // (i) body becomes a task-plan body: the chip flag arms.
    *skill_handle.lock().unwrap() = Some(plan_body.clone());
    let mut flag = false;
    refresh_skill_mirrors(
        &skill_handle,
        &mut active_skill,
        &mut active_skill_body,
        &mut sys_tokens,
        "act",
        &workdir,
        &mut flag,
    );
    assert_eq!(active_skill.as_deref(), Some("task-plan"));
    assert!(flag, "a committed task-plan body lights the chip");

    // (ii) body becomes another skill body: the chip flag disarms.
    *skill_handle.lock().unwrap() = Some(review_body.clone());
    refresh_skill_mirrors(
        &skill_handle,
        &mut active_skill,
        &mut active_skill_body,
        &mut sys_tokens,
        "act",
        &workdir,
        &mut flag,
    );
    assert_eq!(active_skill.as_deref(), Some("review"));
    assert!(!flag, "a non-task-plan commit reverts the hue");

    // (iii-a) body unchanged (early return): a pre-set flag must be preserved.
    let mut flag = true;
    refresh_skill_mirrors(
        &skill_handle,
        &mut active_skill,
        &mut active_skill_body,
        &mut sys_tokens,
        "act",
        &workdir,
        &mut flag,
    );
    assert!(flag, "early return must keep a true caller value");

    // (iii-b) same, preserving a false value.
    let mut flag = false;
    refresh_skill_mirrors(
        &skill_handle,
        &mut active_skill,
        &mut active_skill_body,
        &mut sys_tokens,
        "act",
        &workdir,
        &mut flag,
    );
    assert!(!flag, "early return must keep a false caller value");
}
