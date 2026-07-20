//! P1 contract tests for skill discovery: the public API surface
//! (`discover`, `parse_skill`, `skills_dir`, `Skill` fields), file-layout
//! handling (flat `.md` vs nested `SKILL.md`), frontmatter parsing, and the
//! "missing directory is not an error" guarantee the TUI picker relies on.

use std::fs;

use opencoder_core::skill::{discover_in, parse_skill, seed_builtin_skills_in};
use opencoder_core::{discover_skills, skills_dir, Skill};

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
fn seed_builtin_skills_writes_all_packs_when_gate_absent() {
    let root = tempfile::tempdir().unwrap();
    seed_builtin_skills_in(root.path()).expect("seed");
    let names: Vec<String> = discover_in(root.path())
        .into_iter()
        .map(|s| s.name)
        .collect();
    for expected in ["do-and-done", "repo-local-memory", "review", "submit"] {
        assert!(
            names.iter().any(|n| n == expected),
            "expected seeded skill {expected:?}, got {names:?}"
        );
    }
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
