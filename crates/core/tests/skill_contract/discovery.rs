use super::*;

#[test]
fn skills_dir_points_at_global_home() {
    let _g = ENV_LOCK.lock().unwrap();
    // Isolate the platform's home so this never targets the runner's profile.
    let home = tempfile::tempdir().unwrap();
    let prev_home = std::env::var_os(HOME_ENV);
    std::env::set_var(HOME_ENV, home.path());
    let dir = skills_dir();
    match prev_home {
        Some(h) => std::env::set_var(HOME_ENV, h),
        None => std::env::remove_var(HOME_ENV),
    }

    assert_eq!(
        dir.expect("with a profile set, skills_dir must resolve"),
        home.path().join(".opencoder").join("skills"),
        "skills_dir must be the exact skills directory under the resolved profile"
    );
}

/// No-HOME contract: `skills_dir` never fabricates a *relative* fallback.
/// (`dirs::home_dir` may still resolve a passwd home when `HOME` is unset, so
/// the pinned invariant is "Some(absolute) or None" — the old bug returned a
/// relative `./.opencoder/skills` here, which made seeding WRITE INTO CWD.)
#[test]
fn skills_dir_without_home_is_none_or_absolute_never_cwd() {
    let _g = ENV_LOCK.lock().unwrap();
    let prev_home = std::env::var_os(HOME_ENV);
    std::env::remove_var(HOME_ENV);
    let dir = skills_dir();
    match prev_home {
        Some(h) => std::env::set_var(HOME_ENV, h),
        None => std::env::remove_var(HOME_ENV),
    }
    if let Some(d) = dir {
        assert!(
            d.is_absolute(),
            "skills_dir must never fall back to a relative cwd path: {}",
            d.display()
        );
    }
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
fn parse_skill_frontmatter_only_file_has_empty_body() {
    // Frontmatter-only file: body must be the empty string, NOT the raw
    // file text (the old `raw.trim()` fallback shipped the `---` comment
    // block as if it were instructions).
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("fm-only.md");
    write(&p, "---\nname: fm-only\ndescription: just meta\n---\n");
    let sk = parse_skill(&p, "fm-only").expect("parse");
    assert_eq!(sk.name, "fm-only");
    assert_eq!(sk.description, "just meta");
    assert_eq!(sk.body, "", "frontmatter-only: body stays empty");
}

#[test]
fn parse_skill_strips_bom_and_blank_lines_before_frontmatter() {
    // "UTF-8 with BOM" editors plus stray blank lines must not hide the
    // frontmatter: metadata parses and only the post-fence body remains.
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("bom.md");
    write(
        &p,
        "\u{FEFF}\n\n---\nname: bom\ndescription: bd\n---\nreal body\n",
    );
    let sk = parse_skill(&p, "bom").expect("parse");
    assert_eq!(
        sk.name, "bom",
        "BOM + blank lines must not hide frontmatter"
    );
    assert_eq!(sk.description, "bd");
    assert_eq!(sk.body, "real body", "body is post-fence text only");
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
