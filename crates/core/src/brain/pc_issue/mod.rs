//! Contracts for the first evidence-driven PC diagnosis plan.
mod output;
use anyhow::{ensure, Context, Result};
pub use output::{parse_output, validate_output, validate_transition};
use serde_json::{json, Value};

pub const PLAN_ID: &str = "pc-issue";
pub const STAGES: [&str; 5] = ["impact", "reproduce", "repair", "verify", "conclude"];

pub fn stage(capability: &str) -> Option<&str> {
    capability
        .strip_prefix("pc-issue-")
        .filter(|s| STAGES.contains(s))
}

pub fn capabilities() -> Vec<Value> {
    STAGES.iter().map(|stage| json!({
        "id":format!("pc-issue-{stage}"),"kind":"operator","target":"act",
        "summary":format!("PC issue: {stage}"),
        "input_desc":"problem and settings from root; previous from the immediately preceding successful step (except impact)",
        "output_desc":"pc-issue.stage/v1 JSON with stage, outcome, summary, evidence and stage-specific verified facts",
        "required_inputs":if *stage == "impact" {vec!["problem"]} else {vec!["problem","previous"]},
        "definition":{"name":"act","pc_issue_stage":stage},"version":"1","maturity":"draft"
    })).collect()
}

pub fn plan() -> Value {
    let titles = [
        "影响面定位",
        "Windows 复现取证",
        "隔离修复",
        "构建与复测",
        "证据验收",
    ];
    let objectives = [
        "提交影响面报告：读取原始文本和图片，沿全局索引核对当前源码与版本；无法定位时明确报告证据缺口",
        "提交实机复现评估报告：通过预约 Windows 获取 UI、日志和抓包证据，或明确记录无法执行的原因；有证据的受阻报告也完成本里程碑，不要求必然复现成功",
        "提交修改决策报告：依据实证给出隔离修复、诊断改动、无需修改或阻塞；无充分证据时明确不修改，也完成本里程碑",
        "提交构建复测评估报告：有修改才调用 jy-builder 并实机复测；前序受阻或无需修改时如实报告无需执行或阻塞，也完成本里程碑",
        "核对权威前序结果并提交最终产品结论，包括前后证据及未解决项；证据不足时必须以未解决结论完成报告",
    ];
    json!({"schema_version":7,"title":"PC 问题诊断与修复","objective":"从原始文本和图片出发，以当前源码及 Windows 运行证据定位问题；必要时隔离修复、Team 构建并复测。执行结束不等于产品修复成功。禁止自动合并或发布产品。证据不足时允许以明确未解决结论完成报告，禁止虚构成功。",
        "inputs":{"problem":{"text":"","images":[]},"settings":{"workspace":"/data00/workspace","helper":"/opt/opencoder-pc-issue/current/cli.py","device_node":"","build_node":"","max_repair_rounds":2}},
        "layers":STAGES.iter().enumerate().map(|(i,s)|json!({"layer_id":s,"title":titles[i],"objective":objectives[i],"success_criteria":"以宿主接受的 pc-issue.stage/v1 报告完成里程碑。incomplete、blocked、not_needed、not_reproduced、unresolved 是有效产品结论，不能因此把已完成的报告判为里程碑失败；应向前传递缺口，直至最终报告。"})).collect::<Vec<_>>(),
        "nodes":STAGES.iter().enumerate().map(|(i,s)|json!({"node_id":s,"layer_id":s,"title":titles[i],"objective":objectives[i],"capability_id":format!("pc-issue-{s}")})).collect::<Vec<_>>(),
        "transitions":STAGES.windows(2).map(|pair|json!({"from":pair[0],"to":pair[1],"condition":"当前里程碑已达标，进入下一里程碑"})).chain(std::iter::once(json!({"from":"verify","to":"repair","condition":"复测失败且存在可执行修复、轮次预算尚有余量"}))).collect::<Vec<_>>(),
        "edges":[],"max_rounds":2})
}

pub fn validate_problem(problem: &Value) -> Result<()> {
    let text = problem["text"]
        .as_str()
        .context("problem.text is required")?;
    ensure!(
        !text.trim().is_empty() && text.len() <= 64 * 1024,
        "problem.text must contain 1–65536 bytes"
    );
    let images = problem["images"]
        .as_array()
        .context("problem.images must be an array")?;
    ensure!(images.len() <= 4, "at most four problem images");
    for image in images {
        ensure!(
            image["id"]
                .as_str()
                .is_some_and(|s| s.starts_with("image-")),
            "invalid problem image reference"
        );
        ensure!(
            image["sha256"]
                .as_str()
                .is_some_and(|s| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit())),
            "image sha256 required"
        );
    }
    Ok(())
}

pub fn is_plan(plan: &super::layered::LayeredPlan) -> bool {
    plan.nodes
        .iter()
        .any(|n| n.capability_refs().iter().any(|c| stage(c).is_some()))
}

pub fn validate_plan(plan: &super::layered::LayeredPlan) -> Result<()> {
    if !is_plan(plan) {
        return Ok(());
    }
    ensure!(
        plan.schema_version == 7
            && plan.nodes.len() == STAGES.len()
            && plan.layers.len() == STAGES.len(),
        "PC issue plan requires the complete five-stage evidence chain"
    );
    ensure!(
        (1..=2).contains(&plan.max_rounds),
        "PC issue plan permits at most two rounds"
    );
    for (i, name) in STAGES.iter().enumerate() {
        ensure!(
            plan.layers[i].layer_id == *name
                && plan.nodes.iter().any(|n| n.node_id == *name
                    && n.layer_id == *name
                    && n.capability_id == format!("pc-issue-{name}")),
            "PC issue stage {name} is missing, reordered or rebound"
        );
    }
    ensure!(
        plan.edges.is_empty(),
        "PC issue schema 7 has no legacy node edges"
    );
    let expected: std::collections::BTreeSet<_> = STAGES
        .windows(2)
        .map(|pair| (pair[0], pair[1]))
        .chain(std::iter::once(("verify", "repair")))
        .collect();
    let actual: std::collections::BTreeSet<_> = plan
        .transitions
        .iter()
        .map(|edge| (edge.from.as_str(), edge.to.as_str()))
        .collect();
    ensure!(
        actual == expected && plan.transitions.len() == expected.len(),
        "PC issue transitions must retain the fixed evidence chain"
    );
    Ok(())
}
