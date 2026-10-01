use super::*;

#[test]
fn seeded_task_plan_subagent_skill_requires_delegation_contract() {
    // The companion pack turns a plan into parallel subagent dispatch: it
    // must carry the parent/subagent contract, the write-set isolation rule
    // and its own delegation checklist (progressive disclosure).
    let root = tempfile::tempdir().unwrap();
    seed_builtin_skills_in(root.path()).expect("seed");
    let skill = root.path().join("task-plan-subagent");
    let body = std::fs::read_to_string(skill.join("SKILL.md")).unwrap();
    assert!(body.contains("name: task-plan-subagent"));
    for section in [
        "Parent Agent Contract",
        "Stable Subtask Granularity",
        "Workflow",
        "Output Schema",
        "owner 类型",
        "写入范围",
        "验收方式",
        "回传格式",
        // The parent stays a pure dispatcher: splitting/routing/collecting
        // only, with write-set isolation between parallel subagents.
        "父 agent 只作为 subagent 调度器",
        "写集隔离",
        "references/subagent-delegation-checklist.md",
        // Sole sanctioned cross-skill reference: the companion inherits
        // task-plan's launch-closure checklist as its planning baseline.
        "../task-plan/references/launch-closure-plan-checklist.md",
    ] {
        assert!(
            body.contains(section),
            "task-plan-subagent missing `{section}`"
        );
    }
    // The variant asks the user the same way the planner does, so its own
    // dispatch-decision protocol ships IN the asset (the tool schema stays a
    // one-line pointer). Asking is a PARENT action and is never delegated.
    for guidance in [
        "## 拆分前对齐（question 工具）",
        "不把提问当侦察手段",
        "`question`（string，必填）",
        "`options`（string[]，可选）",
        "同一轮可对多个独立决策点分别调用",
        "工具结果即用户所选答案或原文答复",
        "assumptions:",
        "不得派一个 subagent 去替用户拍板",
    ] {
        assert!(
            body.contains(guidance),
            "task-plan-subagent missing question protocol `{guidance}`"
        );
    }
    let checklist =
        std::fs::read_to_string(skill.join("references/subagent-delegation-checklist.md")).unwrap();
    for section in [
        "## 1. 拆分前检查",
        "## 3. Ownership 与写集隔离",
        "## 6. 最终集成与验证派发",
        "## Subagent Task Schema",
    ] {
        assert!(
            checklist.contains(section),
            "subagent checklist missing `{section}`"
        );
    }
}

#[test]
fn seeded_task_plan_skill_requires_launch_closure_contract() {
    // Task-plan is the codex-isomorphic launch-closure planning full text
    // again: plan-only guard, the `question` clarification protocol inside
    // Overview, When To Use / Scope Rule, Workflow 1..8 (incl. 2.1 contract
    // + freshness matrix), Final Output, Severity Guidance. The deep 4-8.1
    // checklists stay OUT of the bundled reference: the shipped checklist
    // keeps only 1..4 + Plan Output Schema, so progressive disclosure is
    // carried by the SKILL.md workflow itself.
    let root = tempfile::tempdir().unwrap();
    seed_builtin_skills_in(root.path()).expect("seed");
    let body = std::fs::read_to_string(root.path().join("task-plan/SKILL.md")).unwrap();
    assert!(body.contains("name: task-plan"), "frontmatter name missing");
    for contract in [
        "只规划不执行",
        "澄清协议",
        "### 1. 建立规划上下文",
        "### 2. 以上线标准审查当前现状",
        "### 2.1 建立合约与保鲜矩阵",
        "### 3. 提炼根因与缺口地图",
        "### 4. 产出闭环执行规划",
        "### 5. 输出线上或生产等价验证方案",
        "### 6. 做遗漏复查",
        "### 7. 收敛上线路径",
        "### 8. 给出执行结论",
        "## Final Output",
        "## Severity Guidance",
        "证据成熟度",
        "线上 / 生产等价验证",
        "gating item",
        // Final Output field names the closure roadmap must carry.
        "问题定义",
        "现状审查",
        "影响面地图",
        "闭环计划",
        "逐步操作清单",
        "上线路径",
        "最终判断",
        // Progressive-disclosure pointer to the bundled reference.
        "references/launch-closure-plan-checklist.md",
    ] {
        assert!(body.contains(contract), "task-plan missing `{contract}`");
    }
    for severity in ["P0", "P1", "P2", "P3"] {
        assert!(
            body.contains(severity),
            "task-plan missing severity guidance `{severity}`"
        );
    }
    assert!(
        !body.contains("verify-and-summary"),
        "task-plan must not carry the retired workflow step"
    );
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
    // A fresh install must contain exactly the documented reference set.
    let bundled: std::collections::BTreeSet<_> = std::fs::read_dir(&references)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(
        bundled,
        std::collections::BTreeSet::from([std::ffi::OsString::from(
            "launch-closure-plan-checklist.md"
        )]),
        "fresh seeding must not add undocumented references"
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
