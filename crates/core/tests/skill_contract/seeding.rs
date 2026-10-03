use super::*;

#[test]
fn seed_in_writes_all_packs_on_fresh_dir() {
    let root = tempfile::tempdir().unwrap();
    seed_builtin_skills_in(root.path()).expect("seed");
    let names: std::collections::BTreeSet<String> = discover_in(root.path())
        .into_iter()
        .map(|skill| skill.name)
        .collect();
    let expected: std::collections::BTreeSet<String> = [
        "task-plan",
        "task-plan-subagent",
        "do-and-done",
        "repo-local-memory",
        "repo-local-dreaming",
        "say-and-replay",
        "review",
        "summary",
        "submit",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(
        names, expected,
        "fresh installs must seed exactly the built-in packs"
    );
    // repo-local-memory ships sidecar files alongside SKILL.md.
    let rlm = root.path().join("repo-local-memory");
    assert!(rlm.join("EXAMPLES.md").exists());
    assert!(rlm.join("TEMPLATES.md").exists());
    // Codex-standard task-plan uses progressive disclosure: its detailed
    // launch-closure protocol is bundled under references/.
    let task_plan = root.path().join("task-plan");
    assert!(task_plan
        .join("references/launch-closure-plan-checklist.md")
        .exists());
    let task_plan_subagent = root.path().join("task-plan-subagent");
    assert!(task_plan_subagent.join("SKILL.md").exists());
    assert!(task_plan_subagent
        .join("references/subagent-delegation-checklist.md")
        .exists());
}

#[test]
fn seeded_memory_template_uses_a_distinct_index_and_preserves_repository_instructions() {
    let root = tempfile::tempdir().unwrap();
    let repository = root.path().join("repository");
    let instructions = repository.join("AGENTS.md");
    write(
        &instructions,
        "Repository instructions must remain unchanged.\n",
    );
    let skills = root.path().join("skills");
    seed_builtin_skills_in(&skills).unwrap();
    let memory = skills.join("repo-local-memory");
    let template = fs::read_to_string(memory.join("TEMPLATES.md")).unwrap();
    let filename = template
        .lines()
        .find_map(|line| {
            let name = line.strip_prefix("## Template: `")?.strip_suffix('`')?;
            (!name.contains('/')).then_some(name)
        })
        .unwrap();
    assert_eq!(filename, "repo-memory.md");
    assert_ne!(filename.to_ascii_lowercase(), "agents.md");
    write(&repository.join(filename), "Commit: test\nLogic index.\n");
    assert_eq!(
        fs::read_to_string(&instructions).unwrap(),
        "Repository instructions must remain unchanged.\n"
    );
    assert_eq!(
        fs::read_to_string(repository.join(filename)).unwrap(),
        "Commit: test\nLogic index.\n"
    );
    let policy = fs::read_to_string(memory.join("SKILL.md")).unwrap();
    assert!(policy.contains("Preserve `AGENTS.md`"));
    assert!(policy.contains("distinct file from `AGENTS.md`"));
    let dreaming = fs::read_to_string(skills.join("repo-local-dreaming/SKILL.md")).unwrap();
    assert!(dreaming.contains("`repo-memory.md`"));
    assert!(dreaming.contains("保留指令文件"));
}

#[test]
fn seed_builtin_skills_backs_up_then_overwrites_user_edits() {
    let root = tempfile::tempdir().unwrap();
    // Pre-create one skill dir with user-authored content: builtin seeding
    // updates on drift, so the shipped asset wins while the edit is backed
    // up to `<file>.user.bak`.
    let user_file = root.path().join("do-and-done").join("SKILL.md");
    std::fs::create_dir_all(user_file.parent().unwrap()).unwrap();
    std::fs::write(&user_file, "user-authored\n").unwrap();
    // User-authored resources are outside the built-in inventory and must
    // survive seeding without changes or backup copies.
    let user_reference = root.path().join("task-plan/references/custom-contract.md");
    write(&user_reference, "user-reference\n");

    seed_builtin_skills_in(root.path()).expect("seed");

    // Drifted builtin file: ship version wins, user edit backed up...
    assert_ne!(
        std::fs::read_to_string(&user_file).unwrap(),
        "user-authored\n",
        "builtin seed must propagate the shipped asset over drifted content"
    );
    assert_eq!(
        std::fs::read_to_string(&user_file).unwrap(),
        include_str!("../../assets/skills/do-and-done/SKILL.md"),
        "seeded content must equal the freshly shipped asset"
    );
    assert_eq!(
        std::fs::read_to_string(user_file.with_file_name("SKILL.md.user.bak")).unwrap(),
        "user-authored\n",
        "user edit must be backed up before the overwrite"
    );
    // ...while files at paths the built-in no longer ships are never
    // re-seeded built-in content.
    assert_eq!(
        std::fs::read_to_string(&user_reference).unwrap(),
        "user-reference\n"
    );
    assert!(
        !user_reference
            .with_file_name("custom-contract.md.user.bak")
            .exists(),
        "user-authored resources must not be treated as drifted built-ins"
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

    // Existing user file is drifted: ship version wins, edit backed up.
    assert_eq!(
        std::fs::read_to_string(user.with_file_name("SKILL.md.user.bak")).unwrap(),
        "user-authored\n"
    );
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
    std::fs::write(
        root.join("chrome-headless/SKILL.md"),
        "my custom chrome skill",
    )
    .unwrap();

    seed_dep_gated_skills_in(root).unwrap();

    // User file preserved.
    let body = std::fs::read_to_string(root.join("ssh-pty/SKILL.md")).unwrap();
    assert_eq!(body, "my custom skill");

    // chrome-headless user file also preserved.
    let chrome_body = std::fs::read_to_string(root.join("chrome-headless/SKILL.md")).unwrap();
    assert_eq!(chrome_body, "my custom chrome skill");

    // Never-clobber implies no backup churn either.
    assert!(!root.join("ssh-pty/SKILL.md.user.bak").exists());
}
