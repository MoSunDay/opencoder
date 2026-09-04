//! 纯提示词构造：计划/执行两种运行的 LLM 输入完全由这些纯函数从
//! `ProjectContext` 生成——无 IO、无状态，便于单测锁定措辞契约。

/// 一次 plan/execute 运行所需的全部业务上下文（目标→里程碑→待办链路上
/// 各级标题与正文；goal 之外均可缺失，缺失的段落直接省略）。
pub struct ProjectContext {
    pub goal_title: String,
    pub goal_detail_md: Option<String>,
    pub milestone_title: Option<String>,
    pub milestone_detail_md: Option<String>,
    pub todo_title: String,
    pub todo_draft: String,
}

/// 把待办草稿整理成实施方案的提示词。输出契约：只回纯 markdown 计划正文，
/// 不带任何前言、结语或代码围栏包装。
pub fn plan_prompt(cx: &ProjectContext) -> String {
    let mut out = String::new();
    out.push_str("你是一名资深工程规划助手。请把下面这份粗略待办草稿整理成一份完整、可执行的实施方案。\n\n");
    out.push_str("背景：\n");
    out.push_str(&format!("- 目标：{}\n", cx.goal_title));
    if let Some(detail) = &cx.goal_detail_md {
        out.push_str(&format!("  目标说明：{}\n", detail.trim()));
    }
    if let (Some(title), Some(detail)) = (&cx.milestone_title, &cx.milestone_detail_md) {
        out.push_str(&format!("- 里程碑：{}\n", title));
        out.push_str(&format!("  里程碑说明：{}\n", detail.trim()));
    } else if let Some(title) = &cx.milestone_title {
        out.push_str(&format!("- 里程碑：{}\n", title));
    }
    out.push_str(&format!("\n待办：{}\n草稿：{}\n", cx.todo_title, cx.todo_draft.trim()));
    out.push_str(
        "\n要求：\n\
         1. 方案需覆盖实施步骤、涉及文件/模块、验证方式与完成标准，逐条可执行。\n\
         2. 只输出方案本身的 markdown 正文：不要任何开场白、总结语，也不要用代码围栏包裹。\n",
    );
    out
}

/// 按既定方案在工作目录中实际执行的提示词。`resume == true` 表示这是同一
/// 待办的第 N 版续跑（会话上下文延续），否则是首次执行。
pub fn execute_prompt(cx: &ProjectContext, plan_md: &str, version: i64, resume: bool) -> String {
    let mut out = String::new();
    out.push_str("你是编码代理。请依据下面的实施方案，在当前工作目录中立即动手执行。\n\n");
    out.push_str("背景：\n");
    out.push_str(&format!("- 目标：{}\n", cx.goal_title));
    if let Some(title) = &cx.milestone_title {
        out.push_str(&format!("- 里程碑：{}\n", title));
    }
    out.push_str(&format!("- 待办：{}\n", cx.todo_title));
    if resume {
        out.push_str(&format!(
            "\n这是同一 TODO 的第 {} 版执行，继续之前的会话上下文推进：不要重复已完成的工作，从上次进度继续。\n",
            version
        ));
    } else {
        out.push_str(&format!("\n执行版本：第 {} 版。\n", version));
    }
    out.push_str(&format!("\n实施方案（第 {} 版）：\n{}\n", version, plan_md.trim()));
    out.push_str(
        "\n完成标准：方案中的每一项都已实现并验证通过（运行/测试/构建可用），全部完成后简要汇报结果。\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cx() -> ProjectContext {
        ProjectContext {
            goal_title: "构建个人知识库".into(),
            goal_detail_md: Some("长期目标说明".into()),
            milestone_title: Some("M1 检索".into()),
            milestone_detail_md: Some("里程碑说明".into()),
            todo_title: "做一个计数器".into(),
            todo_draft: "先支持自增".into(),
        }
    }

    #[test]
    fn plan_prompt_embeds_full_chain_and_contract() {
        let p = plan_prompt(&cx());
        assert!(p.contains("构建个人知识库"));
        assert!(p.contains("M1 检索"));
        assert!(p.contains("做一个计数器"));
        assert!(p.contains("先支持自增"));
        assert!(p.contains("不要用代码围栏包裹"));
    }

    #[test]
    fn plan_prompt_omits_milestone_section_when_absent() {
        let mut c = cx();
        c.milestone_title = None;
        c.milestone_detail_md = None;
        let p = plan_prompt(&c);
        assert!(!p.contains("里程碑"));
        assert!(p.contains("目标：构建个人知识库"));
    }

    #[test]
    fn execute_prompt_embeds_plan_and_version() {
        let c = cx();
        let p = execute_prompt(&c, "## 步骤\n1. 写代码", 2, false);
        assert!(p.contains("## 步骤\n1. 写代码"));
        assert!(p.contains("第 2 版"));
        assert!(!p.contains("继续之前的会话上下文推进"));
    }

    #[test]
    fn execute_prompt_resume_wording_differs() {
        let c = cx();
        let p = execute_prompt(&c, "plan", 3, true);
        assert!(p.contains("这是同一 TODO 的第 3 版执行，继续之前的会话上下文推进"));
    }
}
