//! P1 contract tests for skill discovery: the public API surface
//! (`discover`, `parse_skill`, `skills_dir`, `Skill` fields), file-layout
//! handling (flat `.md` vs nested `SKILL.md`), frontmatter parsing, and the
//! "missing directory is not an error" guarantee the TUI picker relies on.

use std::fs;

use opencoder_core::skill::{
    discover_in, parse_skill, seed_builtin_skills_in, seed_dep_gated_skills_in,
};
use opencoder_core::{discover_skills, skills_dir, Skill, DEPS_SENTINEL};

fn write(path: &std::path::Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

#[test]
fn skills_dir_points_at_global_home() {
    // Must end with .opencoder/skills (the binary's own config home).
    let dir = skills_dir();
    let s = dir.to_string_lossy();
    assert!(
        s.ends_with(".opencoder/skills"),
        "unexpected skills_dir: {s}"
    );
}

#[test]
fn discover_empty_when_dir_missing() {
    let root = tempfile::tempdir().unwrap();
    let gone = root.path().join("does-not-exist");
    let found = discover_in(&gone);
    assert!(found.is_empty(), "missing dir must yield no skills");
    // The convenience fn delegates to discover_in(skills_dir()); it must never
    // panic even if the user has no ~/.opencoder/skills yet.
    let _ = discover_skills();
}

#[test]
fn discover_reads_flat_md_and_nested_skill_md() {
    let root = tempfile::tempdir().unwrap();
    write(
        &root.path().join("alpha.md"),
        "---\nname: Alpha\ndescription: first skill\n---\nbody-alpha\n",
    );
    write(
        &root.path().join("nested").join("SKILL.md"),
        "nested body line\n",
    );
    let found = discover_in(root.path());
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].name, "Alpha");
    assert_eq!(found[0].description, "first skill");
    assert!(found[0].body.contains("body-alpha"));
    assert_eq!(found[1].name, "nested");
    assert_eq!(found[1].description, "nested body line");
}

#[test]
fn parse_skill_falls_back_to_stem_and_first_line() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("plain.md");
    write(&p, "# Heading\nfirst real line\nmore\n");
    let sk = parse_skill(&p, "plain").expect("parse");
    assert_eq!(sk.name, "plain");
    assert_eq!(sk.description, "first real line");
    assert!(sk.body.contains("first real line"));
}

#[test]
fn parse_skill_blank_frontmatter_name_keeps_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("x.md");
    write(&p, "---\nname:   \ndescription: hi\n---\nbody\n");
    let sk = parse_skill(&p, "x").expect("parse");
    assert_eq!(sk.name, "x");
    assert_eq!(sk.description, "hi");
}

#[test]
fn discover_ignores_non_markdown_files() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("notes.txt"), "not a skill\n");
    write(&dir.path().join("README"), "nope\n");
    let found = discover_in(dir.path());
    assert!(found.is_empty());
}

#[test]
fn skill_fields_are_complete() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("full.md");
    write(&p, "---\nname: Full\ndescription: d\n---\nthe body\n");
    let sk: Skill = parse_skill(&p, "full").unwrap();
    assert_eq!(sk.name, "Full");
    assert_eq!(sk.description, "d");
    assert!(sk.body.contains("the body"));
    assert_eq!(sk.source, p);
}

#[test]
fn seed_in_writes_all_packs_on_fresh_dir() {
    let root = tempfile::tempdir().unwrap();
    seed_builtin_skills_in(root.path()).expect("seed");
    let names: Vec<String> = discover_in(root.path())
        .into_iter()
        .map(|s| s.name)
        .collect();
    for expected in [
        "task-plan",
        "do-and-done",
        "repo-local-memory",
        "repo-local-dreaming",
        "say-and-replay",
        "review",
        "summary",
        "submit",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "expected seeded skill {expected:?}, got {names:?}"
        );
    }
    // Retired built-ins must stay retired: upgrades must not re-seed packs
    // that were removed from BUILTIN_SKILLS (e.g. fk-cli).
    assert!(
        !names.iter().any(|n| n == "fk-cli"),
        "fk-cli must no longer be seeded, got {names:?}"
    );
    // repo-local-memory ships sidecar files alongside SKILL.md.
    let rlm = root.path().join("repo-local-memory");
    assert!(rlm.join("EXAMPLES.md").exists());
    assert!(rlm.join("TEMPLATES.md").exists());
}

#[test]
fn seed_builtin_skills_does_not_clobber_existing_files() {
    let root = tempfile::tempdir().unwrap();
    // Pre-create one skill dir with user-authored content.
    let user_file = root.path().join("do-and-done").join("SKILL.md");
    std::fs::create_dir_all(user_file.parent().unwrap()).unwrap();
    std::fs::write(&user_file, "user-authored\n").unwrap();

    seed_builtin_skills_in(root.path()).expect("seed");

    // Existing user file must be preserved...
    assert_eq!(
        std::fs::read_to_string(&user_file).unwrap(),
        "user-authored\n"
    );
    // ...while the other packs are still written.
    assert!(root.path().join("review").join("SKILL.md").exists());
}

#[test]
fn seed_in_adds_missing_skills_to_partial_dir() {
    // Regression: previously a gate on `review` dir existing caused
    // seed_builtin_skills to early-return, so a binary upgrade that ships a
    // new built-in skill never landed it for existing installs. The writer
    // core is now purely incremental: missing skills are added, existing
    // files are untouched.
    let root = tempfile::tempdir().unwrap();
    let r = root.path();

    // Simulate an existing install that has the old gate skill + a user edit.
    let user = r.join("do-and-done").join("SKILL.md");
    std::fs::create_dir_all(user.parent().unwrap()).unwrap();
    std::fs::write(&user, "user-authored\n").unwrap();
    // `review` present — this was the old gate that short-circuited seeding.
    std::fs::create_dir_all(r.join("review")).unwrap();

    seed_builtin_skills_in(r).expect("seed");

    // Existing user file preserved (never clobbered).
    assert_eq!(std::fs::read_to_string(&user).unwrap(), "user-authored\n");
    // Skills that were missing are now written — including ones added after
    // the original install.
    assert!(r.join("task-plan").join("SKILL.md").exists());
    assert!(r.join("summary").join("SKILL.md").exists());
    assert!(r.join("review").join("SKILL.md").exists());
}

#[test]
fn seed_dep_gated_skills_only_when_sentinel() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    // Without sentinel: dep-gated skills should NOT be seeded.
    seed_dep_gated_skills_in(root).unwrap();
    assert!(!root.join("ssh-pty").exists());

    // With sentinel: dep-gated skills SHOULD be seeded.
    std::fs::write(root.join(DEPS_SENTINEL), "").unwrap();
    seed_dep_gated_skills_in(root).unwrap();
    assert!(root.join("ssh-pty/SKILL.md").exists());

    // Content should be non-empty.
    let ssh_body = std::fs::read_to_string(root.join("ssh-pty/SKILL.md")).unwrap();
    assert!(ssh_body.contains("ssh-pty"));
    assert!(ssh_body.contains("ssh_pty"));
}

#[test]
fn dep_gated_skills_do_not_clobber_existing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join(DEPS_SENTINEL), "").unwrap();

    // Pre-write a user-modified ssh-pty skill.
    std::fs::create_dir_all(root.join("ssh-pty")).unwrap();
    std::fs::write(root.join("ssh-pty/SKILL.md"), "my custom skill").unwrap();

    // Pre-write a user-modified chrome-headless skill too.
    std::fs::create_dir_all(root.join("chrome-headless")).unwrap();
    std::fs::write(root.join("chrome-headless/SKILL.md"), "my custom chrome skill").unwrap();

    seed_dep_gated_skills_in(root).unwrap();

    // User file preserved.
    let body = std::fs::read_to_string(root.join("ssh-pty/SKILL.md")).unwrap();
    assert_eq!(body, "my custom skill");

    // chrome-headless user file also preserved.
    let chrome_body = std::fs::read_to_string(root.join("chrome-headless/SKILL.md")).unwrap();
    assert_eq!(chrome_body, "my custom chrome skill");
}

#[test]
fn body_with_source_emits_path_annotation_then_body() {
    // After discovery, a skill's body_with_source must carry the on-disk path
    // of its source SKILL.md so the agent can locate sibling assets.
    let root = tempfile::tempdir().unwrap();
    write(
        &root.path().join("demo").join("SKILL.md"),
        "---\nname: demo\ndescription: d\n---\nSee [EXAMPLES](./EXAMPLES.md)\n",
    );
    let found = discover_in(root.path());
    assert_eq!(found.len(), 1);
    let sk = &found[0];
    let annotated = opencoder_core::body_with_source(sk);
    let source_str = sk.source.to_string_lossy();
    assert!(
        annotated.starts_with(&format!("> Source: {}", source_str)),
        "annotation must start with the resolved source path: {annotated}"
    );
    assert!(
        annotated.contains("See [EXAMPLES](./EXAMPLES.md)"),
        "body content must follow annotation: {annotated}"
    );
}

#[test]
fn seeded_review_skill_requires_five_question_recap() {
    // The built-in review skill is embedded via include_str! and seeded on
    // first run; its REVIEW block must carry the five-question recap fields
    // (goal / progress / done+verify / blockers / next_todos) so a dropped
    // field in the markdown asset turns this test red.
    let root = tempfile::tempdir().unwrap();
    seed_builtin_skills_in(root.path()).expect("seed");
    let body = std::fs::read_to_string(root.path().join("review/SKILL.md")).unwrap();
    assert!(body.contains("name: review"), "frontmatter name missing");
    assert!(
        body.contains("description:"),
        "frontmatter description missing"
    );
    for field in [
        "goal:",
        "progress:",
        "done:",
        "verify:",
        "blockers:",
        "next_todos:",
    ] {
        assert!(body.contains(field), "review skill missing `{field}`");
    }
    assert!(
        body.contains("progress: <completed 数>/<总数>（<0-100>%"),
        "review REVIEW block must quantify progress (completed/total + percent)"
    );
}

#[test]
fn seeded_say_and_replay_skill_requires_five_question_recap() {
    // Same guard for the say-and-replay REPLAY block: goal / progress /
    // done+verify / encountered + blocked / remaining must all survive
    // asset edits.
    let root = tempfile::tempdir().unwrap();
    seed_builtin_skills_in(root.path()).expect("seed");
    let body =
        std::fs::read_to_string(root.path().join("say-and-replay/SKILL.md")).unwrap();
    assert!(
        body.contains("name: say-and-replay"),
        "frontmatter name missing"
    );
    assert!(
        body.contains("description:"),
        "frontmatter description missing"
    );
    for field in [
        "goal:",
        "progress:",
        "verify:",
        "encountered:",
        "blocked:",
        "remaining:",
    ] {
        assert!(
            body.contains(field),
            "say-and-replay skill missing `{field}`"
        );
    }
    assert!(
        body.contains("（<0-100>%"),
        "say-and-replay progress must carry an explicit percent, not a bare ratio"
    );
    assert!(
        body.contains("百分比"),
        "say-and-replay field semantics must explain the percent convention"
    );
}
