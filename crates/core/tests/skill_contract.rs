//! P1 contract tests for skill discovery: the public API surface
//! (`discover`, `parse_skill`, `skills_dir`, `Skill` fields), file-layout
//! handling (flat `.md` vs nested `SKILL.md`), frontmatter parsing, and the
//! "missing directory is not an error" guarantee the TUI picker relies on.

use std::fs;
use std::sync::Mutex;

use opencoder_core::skill::{
    discover_in, parse_skill, seed_builtin_skills_in, seed_dep_gated_skills_in,
};
use opencoder_core::{discover_skills, skills_dir, Skill, DEPS_SENTINEL};

// Env mutation is process-global; serialize the HOME-manipulating tests.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn write(path: &std::path::Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

#[test]
fn skills_dir_points_at_global_home() {
    let _g = ENV_LOCK.lock().unwrap();
    // Isolate HOME so the assertion targets the temp home, not the runner's.
    let home = tempfile::tempdir().unwrap();
    let prev_home = std::env::var_os("HOME");
    std::env::set_var("HOME", home.path());
    let dir = skills_dir();
    match prev_home {
        Some(h) => std::env::set_var("HOME", h),
        None => std::env::remove_var("HOME"),
    }

    let dir = dir.expect("with HOME set, skills_dir must resolve");
    let s = dir.to_string_lossy();
    assert!(
        s.ends_with(".opencoder/skills"),
        "unexpected skills_dir: {s}"
    );
    assert!(
        dir.starts_with(home.path()),
        "skills_dir must live under the resolved home: {s}"
    );
}

/// No-HOME contract: `skills_dir` never fabricates a *relative* fallback.
/// (`dirs::home_dir` may still resolve a passwd home when `HOME` is unset, so
/// the pinned invariant is "Some(absolute) or None" — the old bug returned a
/// relative `./.opencoder/skills` here, which made seeding WRITE INTO CWD.)
#[test]
fn skills_dir_without_home_is_none_or_absolute_never_cwd() {
    let _g = ENV_LOCK.lock().unwrap();
    let prev_home = std::env::var_os("HOME");
    std::env::remove_var("HOME");
    let dir = skills_dir();
    match prev_home {
        Some(h) => std::env::set_var("HOME", h),
        None => std::env::remove_var("HOME"),
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
    // Codex-standard task-plan uses progressive disclosure: its detailed
    // launch-closure protocol is bundled under references/.
    let task_plan = root.path().join("task-plan");
    assert!(task_plan
        .join("references/launch-closure-plan-checklist.md")
        .exists());
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
    // A user file at a path the built-in no longer ships: seeding must leave
    // user files alone AND must not re-seed the removed Any Home reference,
    // so the stale user copy survives untouched (neither overwritten nor
    // deleted).
    let user_reference = root
        .path()
        .join("task-plan/references/any-home-plan-run.md");
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
        include_str!("../assets/skills/do-and-done/SKILL.md"),
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
    assert_ne!(
        std::fs::read_to_string(&user_reference).unwrap(),
        "# Any Home task-plan protocol\n",
        "seeding must not restore built-in content for the removed reference"
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
    // Review answers five mandatory questions — answering them well IS the
    // output (no fixed output template) — then rules go-live readiness.
    // The review is a pure logic review: it checks the change's own logic
    // plus the logic impact on modules the change could touch, and never
    // demands test/regression execution.
    let root = tempfile::tempdir().unwrap();
    seed_builtin_skills_in(root.path()).expect("seed");
    let body = std::fs::read_to_string(root.path().join("review/SKILL.md")).unwrap();
    assert!(body.contains("name: review"), "frontmatter name missing");
    assert!(
        body.contains("description:"),
        "frontmatter description missing"
    );
    for section in [
        "问一：原始需求目标",
        "问二：做了哪些事情及完成度",
        "问三：卡点",
        "问四：逐项逻辑核查",
        "问五：下一步 TODO",
        "## 上线结论",
    ] {
        assert!(body.contains(section), "review skill missing `{section}`");
    }
    assert!(
        body.contains("completed/total") && body.contains("向下取整"),
        "review must quantify completion as completed/total with a floored percentage"
    );
    assert!(
        body.contains("逻辑本身") && body.contains("变更潜在影响"),
        "review must focus on the change's own logic and the logic impact on affected modules"
    );
    assert!(
        !body.contains("当次实跑") && !body.contains("复测"),
        "review must not demand fresh test/regression runs as evidence"
    );
    assert!(
        body.contains("go-live ready") && body.contains("not ready"),
        "review must rule go-live readiness"
    );
    assert!(
        !body.contains("Output Shape") && !body.contains("goal:"),
        "review must not carry the retired REVIEW output template"
    );
}

#[test]
fn seeded_say_and_replay_skill_requires_five_question_recap() {
    // Same guard for the say-and-replay REPLAY block: goal / progress /
    // done+verify / encountered + blocked / remaining must all survive
    // asset edits.
    let root = tempfile::tempdir().unwrap();
    seed_builtin_skills_in(root.path()).expect("seed");
    let body = std::fs::read_to_string(root.path().join("say-and-replay/SKILL.md")).unwrap();
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

#[test]
fn seeded_task_plan_skill_requires_launch_closure_contract() {
    // Task-plan delivers ONE plan-only closure roadmap whose output covers
    // five anchors (goal, key context, TODO list, per-TODO verification,
    // key-path actions), grades evidence without cross-level inference,
    // audits omissions, and discloses hard blockers. Deep contract/freshness
    // detail is progressively disclosed via the bundled checklist reference.
    let root = tempfile::tempdir().unwrap();
    seed_builtin_skills_in(root.path()).expect("seed");
    let body = std::fs::read_to_string(root.path().join("task-plan/SKILL.md")).unwrap();
    assert!(body.contains("name: task-plan"), "frontmatter name missing");
    for contract in [
        "树立目标",
        "关键 context",
        "TODO List",
        "TODO 验证手段",
        "核心动作",
        "证据成熟度",
        "线上 / 生产等价验证",
        "做遗漏复查",
        "gating item",
    ] {
        assert!(body.contains(contract), "task-plan missing `{contract}`");
    }
    let references = root.path().join("task-plan/references");
    let checklist =
        std::fs::read_to_string(references.join("launch-closure-plan-checklist.md")).unwrap();
    // Sections 4-8.1 (feature/data-security/regression/launch-window/
    // freshness checklists) were deliberately removed from the shipped
    // reference: the five anchors in SKILL.md already cover verification,
    // and the deep release-window/freshness detail bloated every plan.
    assert!(
        !checklist.contains("持续保鲜与稳定性"),
        "removed freshness section must not re-seed into the checklist"
    );
    // Pinned WITH section numbers: kept sections stay contiguous 1..4
    // (renumbered after 4-8.1 removal — no gap like the stale `## 9.`).
    for kept in [
        "## 1. 需求与现状审查",
        "## 2. 根因与缺口识别",
        "## 3. 代码与模块影响",
        "## 4. 遗漏复查与交付可读性",
        "## Plan Output Schema",
    ] {
        assert!(
            checklist.contains(kept),
            "checklist missing kept section `{kept}`"
        );
    }
    // Fresh-seed contract: the retired Any Home protocol must NOT be
    // re-seeded into a fresh install, while the launch-closure checklist
    // (its replacement reference) still lands.
    assert!(
        !references.join("any-home-plan-run.md").exists(),
        "removed Any Home reference must not be re-seeded into a fresh install"
    );
    assert!(
        references.join("launch-closure-plan-checklist.md").exists(),
        "launch-closure checklist must be bundled"
    );
}

#[test]
fn seeded_task_plan_skill_requires_question_tool_guidance() {
    // Regression guard (restored after 04df804 dropped it): the `question`
    // tool is unlocked by the task-plan skill itself (latent, behind the
    // 500-char window gate) and is the sanctioned clarification channel
    // under act/sandbox alike. The skill must keep: (a) the conditional
    // protocol (interactive -> ask via `question`, one key question per
    // call, several per turn; headless -> explicit `assumptions:`), and
    // (b) the anti-lazy guard (repo/rules/test facts are looked up, not
    // asked). Ambiguity must never turn into silently invented acceptance
    // criteria.
    let root = tempfile::tempdir().unwrap();
    seed_builtin_skills_in(root.path()).expect("seed");
    let body = std::fs::read_to_string(root.path().join("task-plan/SKILL.md")).unwrap();
    assert!(body.contains("name: task-plan"), "frontmatter name missing");
    for guidance in [
        "澄清协议",
        "question",
        "同一轮可对多个独立决策点分别调用",
        "不把提问当侦察手段",
        "assumptions:",
    ] {
        assert!(body.contains(guidance), "task-plan missing `{guidance}`");
    }
    // Full parameter + usage contract lives IN the skill text (the user's
    // single documentation surface); the tool JSON schema stays minimal.
    for usage in [
        "`question`（string，必填）",
        "`options`（string[]，可选）",
        r#"调用示例：`{"question":"#,
        "工具结果即用户所选答案或原文答复",
    ] {
        assert!(body.contains(usage), "task-plan missing `{usage}`");
    }
}

/// The `question` tool is latent and unlocked from the FIRST 500 chars of a
/// skill body (session-side `tools::latent::unlocked_from_body`). The
/// `task-plan` seed — its only owner — must therefore name ITSELF and the
/// `question` tool inside that window, or activating the skill silently
/// leaves the clarification tool hidden from the model.
#[test]
fn seeded_task_plan_body_unlocks_question_in_prefix_window() {
    let root = tempfile::tempdir().unwrap();
    seed_builtin_skills_in(root.path()).expect("seed");
    let skill = "task-plan";
    let path = root.path().join(skill).join("SKILL.md");
    let raw = std::fs::read_to_string(&path).unwrap();
    let parsed = parse_skill(&path, "fallback").expect("seeded skill parses");
    // The unlock sees the injected body (source path + frontmatter-stripped
    // body); mirror that here.
    let injected = format!("> Source: {}\n\n{}", parsed.source.display(), parsed.body);
    let prefix: String = injected.chars().take(500).collect();
    assert!(
        prefix.contains(skill),
        "task-plan body must name itself within the first 500 chars"
    );
    assert!(
        prefix.contains("question"),
        "task-plan body must mention the question tool within the first 500 chars"
    );
    assert!(
        raw.contains("不把提问当侦察手段") && raw.contains("assumptions:"),
        "task-plan must keep the lookup-first guard and the headless assumptions fallback"
    );
}

/// `question` is task-plan-only now: the `review` seed must neither promise
/// the interactive question flow nor carry the literal `task-plan` token
/// (its own name-match in the 500-char prefix window would silently hijack
/// the unlock). Session-side tests pin the actual unlock behavior; this
/// guards the seed asset itself.
#[test]
fn seeded_review_skill_requires_no_question_tool() {
    let root = tempfile::tempdir().unwrap();
    seed_builtin_skills_in(root.path()).expect("seed");
    let body = std::fs::read_to_string(root.path().join("review/SKILL.md")).unwrap();
    assert!(body.contains("name: review"), "frontmatter name missing");
    for guidance in [
        "不把提问当侦察手段",
        "assumptions:",
        "不调用 `question` 工具",
    ] {
        assert!(body.contains(guidance), "review missing `{guidance}`");
    }
    assert!(
        !body.contains("可在同一轮多问"),
        "review must not promise the interactive multi-question flow"
    );
    assert!(
        !body.contains("task-plan"),
        "review must not carry the task-plan token (it would hijack the question unlock)"
    );
}

#[test]
fn seeded_workflow_skills_consume_launch_closure_plan() {
    // The Codex-standard task-plan no longer emits the legacy fixed STATUS
    // block. Direct consumers must follow its closure-plan/evidence contract
    // instead of waiting for fields the planner will never produce.
    let root = tempfile::tempdir().unwrap();
    seed_builtin_skills_in(root.path()).expect("seed");
    for skill in ["do-and-done", "summary", "submit"] {
        let body = std::fs::read_to_string(root.path().join(skill).join("SKILL.md")).unwrap();
        assert!(
            !body.contains("STATUS 块"),
            "{skill} still requires the retired STATUS block"
        );
    }
    let executor = std::fs::read_to_string(root.path().join("do-and-done/SKILL.md")).unwrap();
    assert!(
        executor.contains("闭环计划") && executor.contains("go-live ready"),
        "executor must drive the closure plan through fresh review"
    );
}

#[test]
fn seeded_submit_skill_consumes_review_logic_recap() {
    // submit consumes review's per-item logic recap (pure logic review, no
    // run evidence since the 2026-09-02 logic-only retune); the gate-green
    // premise is anchored to do-and-done's own verification plus the
    // rules/02 iteration regression gate, not to review-run evidence.
    let root = tempfile::tempdir().unwrap();
    seed_builtin_skills_in(root.path()).expect("seed");
    let body = std::fs::read_to_string(root.path().join("submit/SKILL.md")).unwrap();
    assert!(
        body.contains("逐项逻辑核查与影响面汇总"),
        "submit must consume review's logic recap, not a run-evidence summary"
    );
    assert!(
        !body.contains("证据汇总"),
        "submit must not reference the retired review run-evidence summary"
    );
    assert!(
        body.contains("go-live ready") && body.contains("rules/02") && body.contains("迭代回归"),
        "submit's gate-green premise must cite do-and-done verification + rules/02 iteration gate"
    );
}
