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
        "读取原始文本和图片，沿全局索引核对当前源码、版本和潜在影响面",
        "通过设备预约执行原始问题并获取真实 UI、日志和抓包证据",
        "根据实证决定最小修复、诊断改动或无需修改",
        "有修改才调用 jy-builder 构建候选包，并在预约 Windows 上复测",
        "核对权威前序结果并报告产品结论、前后证据及未解决项",
    ];
    json!({"schema_version":5,"title":"PC 问题诊断与修复","objective":"从原始文本和图片出发，以当前源码及 Windows 运行证据定位问题；必要时隔离修复、Team 构建并复测。执行结束不等于产品修复成功。禁止自动合并或发布产品。证据不足时允许以明确未解决结论完成报告，禁止虚构成功。",
        "inputs":{"problem":{"text":"","images":[]},"settings":{"workspace":"/data00/workspace","helper":"/opt/opencoder-pc-issue/current/cli.py","device_node":"","build_node":"","max_repair_rounds":2}},
        "nodes":STAGES.iter().enumerate().map(|(i,s)|json!({"node_id":s,"layer":i+1,"title":titles[i],"objective":objectives[i],"success_criteria":"输出通过 pc-issue.stage/v1 校验的真实结果和证据；明确区分产品成功、未复现、无需修改和阻塞。构建复测失败且有可执行修复时反思回退；达到两轮后保留失败证据进入最终报告。","capability_ids":[format!("pc-issue-{s}")]})).collect::<Vec<_>>(),
        "edges":[{"from":"verify","to":"repair","condition":"候选复测失败且存在证据充分的修复方案，尚未达到两轮预算"}],"max_rounds":2})
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
        .any(|n| n.capability_ids.iter().any(|c| stage(c).is_some()))
}

pub fn validate_plan(plan: &super::layered::LayeredPlan) -> Result<()> {
    if !is_plan(plan) {
        return Ok(());
    }
    ensure!(
        plan.schema_version == 5 && plan.nodes.len() == STAGES.len(),
        "PC issue plan requires the complete five-stage evidence chain"
    );
    ensure!(
        (1..=2).contains(&plan.max_rounds),
        "PC issue plan permits at most two rounds"
    );
    for (i, name) in STAGES.iter().enumerate() {
        ensure!(
            plan.nodes.iter().any(|n| n.node_id == *name
                && n.layer == i as u32 + 1
                && n.capability_ids == [format!("pc-issue-{name}")]),
            "PC issue stage {name} is missing, reordered or rebound"
        );
    }
    ensure!(
        plan.edges.len() <= 1
            && plan
                .edges
                .iter()
                .all(|e| e.from == "verify" && e.to == "repair"),
        "PC issue reflection must return verification to repair"
    );
    Ok(())
}
