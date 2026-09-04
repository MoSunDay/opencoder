//! Pure prompt builders (Chinese). Decision prompts always state 只输出 JSON
//! because the replies are machine-parsed; free-text member replies are not.

use crate::types::{ResultRecord, TeamMember};

/// One completed turn, as the captain sees it in later prompts.
#[derive(Debug, Clone)]
pub struct TurnDigest {
    pub turn: usize,
    pub question: String,
    pub aligned: bool,
    pub summary: String,
}

/// 截断原始回复，用于纠错重问（避免把超长垃圾全文塞回 prompt）。
pub fn truncate(raw: &str, max_chars: usize) -> String {
    if raw.chars().count() <= max_chars {
        return raw.to_string();
    }
    let cut: String = raw.chars().take(max_chars).collect();
    format!("{cut}…[截断]")
}

fn member_table(members: &[TeamMember]) -> String {
    members
        .iter()
        .map(|m| {
            let caps = if m.capabilities.is_empty() {
                "未知".to_string()
            } else {
                m.capabilities.join("；")
            };
            format!("- node_id: {}（{}）擅长：{}", m.node_id, m.name, caps)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn history_block(history: &[TurnDigest]) -> String {
    if history.is_empty() {
        return "（这是第一轮讨论）".to_string();
    }
    history
        .iter()
        .map(|t| {
            format!(
                "第 {} 轮：问题「{}」，对齐：{}，小结：{}",
                t.turn,
                t.question,
                if t.aligned { "是" } else { "否" },
                t.summary
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 决策①：队长规划下一轮的问题与参与成员。
pub fn plan_prompt(
    requirement: &str,
    members: &[TeamMember],
    history: &[TurnDigest],
    next_question_hint: Option<&str>,
) -> String {
    let hint = next_question_hint
        .map(|q| format!("上一轮建议的下一问题：{q}"))
        .unwrap_or_else(|| "无".to_string());
    format!(
        "你是团队队长。请根据需求、成员能力与历史讨论，规划下一轮讨论的核心问题，并选出回答该问题的成员。\n\
         要求：\n\
         1. question：一个具体、值得多成员交叉讨论的问题。\n\
         2. participants：参与成员的 node_id 数组；只能从下列成员中选择，至少 1 人，不得重复。\n\
         3. rationale：一句话说明为什么问这个问题、为什么选这些人。\n\
         只输出 JSON，不要 Markdown 围栏，不要任何解释文字，格式：\
         {{\"question\":\"...\",\"participants\":[\"node_id\"],\"rationale\":\"...\"}}\n\n\
         需求：{requirement}\n\n成员列表：\n{members}\n\n历史讨论：\n{history}\n\n{hint}",
        members = member_table(members),
        history = history_block(history)
    )
}

/// 成员作答（自由文本，不作 JSON 要求）。
pub fn answer_prompt(requirement: &str, question: &str, member_id: &str) -> String {
    format!(
        "你是团队成员 {member_id}。请针对下面的问题给出你的专业回答：直接给出结论、依据与必要的取舍说明，\
         简洁明确，不要重复问题本身。\n\n需求背景：{requirement}\n\n本轮问题：{question}"
    )
}

/// 对齐追答：上一轮总结后该成员仍有歧义，需针对性澄清（自由文本）。
pub fn alignment_prompt(
    requirement: &str,
    question: &str,
    summary: &str,
    ambiguity_question: &str,
) -> String {
    format!(
        "团队队长认为你上一轮的回答仍未与团队对齐，需要你针对性澄清。\n\
         请直接回答需要澄清的点：明确你的立场、与其他成员的差异点是什么、你坚持或让步的理由。\n\n\
         需求背景：{requirement}\n本轮问题：{question}\n团队当前小结：{summary}\n\n需要你澄清：{ambiguity_question}"
    )
}

/// 决策②：队长汇总本轮结果并判定是否对齐。ok=false 的成员以失败标注呈现。
pub fn summary_prompt(requirement: &str, question: &str, results: &[ResultRecord]) -> String {
    let lines = results
        .iter()
        .map(|r| {
            if r.ok {
                format!("- {}：{}", r.node_id, r.answer)
            } else {
                format!(
                    "- {}：（回答失败：{}）",
                    r.node_id,
                    r.error.as_deref().unwrap_or("未知错误")
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "你是团队队长。请汇总本轮各成员的回答，判断团队是否已对齐。\n\
         要求：\n\
         1. summary：本轮讨论小结（观点、共识、分歧）。\n\
         2. aligned：布尔值，成员观点是否已对齐到足以推进。\n\
         3. ambiguities：仍需澄清的成员列表，每项 {{\"node_id\":\"...\",\"question\":\"需要澄清的具体问题\"}}；\
         已对齐或无成员需要澄清时为空数组，不要罗列已说清的成员。\n\
         只输出 JSON，不要 Markdown 围栏，不要任何解释文字，格式：\
         {{\"summary\":\"...\",\"aligned\":true,\"ambiguities\":[{{\"node_id\":\"...\",\"question\":\"...\"}}]}}\n\n\
         需求背景：{requirement}\n本轮问题：{question}\n成员回答：\n{lines}"
    )
}

/// 决策③：收尾判断。complete=true 时 final_summary 必填。
pub fn closing_prompt(requirement: &str, history: &[TurnDigest]) -> String {
    format!(
        "你是团队队长。请基于全部轮次的讨论判断话题是否可以收尾。\n\
         要求：\n\
         1. complete：布尔值，讨论是否已充分、可以给出最终结论。\n\
         2. next_question：complete=false 时给出下一轮建议讨论的问题；complete=true 时可为 null。\n\
         3. final_summary：complete=true 时必填，给出面向需求的最终结论（综合全部轮次）；否则为 null。\n\
         只输出 JSON，不要 Markdown 围栏，不要任何解释文字，格式：\
         {{\"complete\":true,\"next_question\":null,\"final_summary\":\"...\"}}\n\n\
         需求：{requirement}\n\n各轮讨论：\n{history}",
        history = history_block(history)
    )
}

/// 能力画像：成员自述擅长领域。
pub fn profile_prompt() -> String {
    "请自述你的能力画像：你擅长的领域、工具、编程语言与工程角色。\
     每条能力用一句话描述，给出 3~6 条。\
     只输出 JSON，不要 Markdown 围栏，不要任何解释文字，格式：\
     {\"capabilities\":[\"一句话一条\",...]}"
        .to_string()
}

/// 纠错重问（todos 风格）：带上解析错误与原始回复截断。
pub fn correction_prompt(error: &str, raw_reply: &str) -> String {
    format!(
        "你上一条回复无法按要求的 JSON 格式解析（错误：{error}）。\n\
         你的原始回复（截断）：{raw}\n\
         请重新回复：只输出一个合法的 JSON 对象，不要 Markdown 围栏，不要任何解释文字，\
         字段与上一条消息中的要求完全一致。",
        raw = truncate(raw_reply, 400)
    )
}
