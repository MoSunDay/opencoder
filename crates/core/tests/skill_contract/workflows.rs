use super::*;

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
fn seeded_review_skill_enforces_lean_output_contract() {
    // The review output is strictly five questions + verdict, nothing else:
    // progress (completed/total), a numbered TODO list, per-item verify
    // method + evidence, and an explicit anti-redundancy discipline replace
    // the retired "no fixed output template" free-form guidance.
    let root = tempfile::tempdir().unwrap();
    seed_builtin_skills_in(root.path()).expect("seed");
    let body = std::fs::read_to_string(root.path().join("review/SKILL.md")).unwrap();
    for token in [
        "## 输出契约",
        "板块之外零输出",
        "严格五问",
        "TODO List",
        "编号",
        "验证方式",
        "证据 = `file:line`",
        "不整段粘贴",
        "不写开篇引言",
        "不跨板块复读",
    ] {
        assert!(
            body.contains(token),
            "review lean output contract missing `{token}`"
        );
    }
    assert!(
        !body.contains("没有固定输出模板"),
        "review must drop the retired free-form (no fixed template) guidance"
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
    for guidance in ["不把提问当侦察手段", "assumptions:", "不向用户提问"] {
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

/// Built-in skills are SELF-CONTAINED: no skill asset may carry another
/// built-in skill's name — unless a documented companion has an intentional
/// parent contract (the sole permitted reference is task-plan-subagent ->
/// task-plan, which inherits its launch-closure checklist). The
/// plan -> execute -> review -> submit workflow is orchestrated by the
/// caller / system prompt, never encoded inside the skills themselves
/// (不要在 skill 里写 skill 衔接). A stray cross-skill
/// token is also an unlock hazard: `task-plan` inside another body's
/// Source-less 500-char prefix would silently hijack the latent `question`
/// unlock (see session-side `tools::latent`).
#[test]
fn seeded_builtin_skills_carry_no_cross_skill_references() {
    let root = tempfile::tempdir().unwrap();
    seed_builtin_skills_in(root.path()).expect("seed builtins");
    std::fs::write(root.path().join(DEPS_SENTINEL), "").unwrap();
    seed_dep_gated_skills_in(root.path()).expect("seed dep-gated");

    let names = [
        "task-plan",
        "task-plan-subagent",
        "do-and-done",
        "repo-local-memory",
        "repo-local-dreaming",
        "say-and-replay",
        "review",
        "submit",
        "summary",
        "ssh-pty",
        "chrome-headless",
    ];
    for skill in names {
        let mut stack = vec![root.path().join(skill)];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }
                let body = std::fs::read_to_string(&path).unwrap();
                for other in names {
                    if other == skill {
                        continue;
                    }
                    if skill == "task-plan-subagent" && other == "task-plan" {
                        continue;
                    }
                    assert!(
                        !body.contains(other),
                        "{skill} asset {:?} must not reference `{other}`: skills stay self-contained",
                        path.strip_prefix(root.path()).unwrap()
                    );
                }
            }
        }
    }
}

/// The `question` tool is documented in exactly ONE place: the planning pair
/// — `task-plan` and its delegation variant `task-plan-subagent`, the only
/// two skills that unlock it. Every other built-in skill must stay silent
/// about the tool — describing it elsewhere teaches models to call a tool
/// that remains latent outside planning context.
#[test]
fn seeded_question_tool_docs_live_only_in_the_planning_pair() {
    let root = tempfile::tempdir().unwrap();
    seed_builtin_skills_in(root.path()).expect("seed");
    for skill in [
        "do-and-done",
        "repo-local-memory",
        "repo-local-dreaming",
        "say-and-replay",
        "review",
        "submit",
        "summary",
    ] {
        let body = std::fs::read_to_string(root.path().join(skill).join("SKILL.md")).unwrap();
        assert!(
            !body.contains("question 工具") && !body.contains("`question`"),
            "{skill} must not describe the `question` tool \
             (doc surface = the planning pair task-plan / task-plan-subagent)"
        );
    }
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
