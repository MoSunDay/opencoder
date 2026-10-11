//! Operator policy formatting is pure; directory resolution stays at the edge.
use std::path::PathBuf;

pub(super) fn render(kind: Option<&str>, roots: &[PathBuf]) -> Option<String> {
    if kind != Some("operator") {
        return None;
    }
    let mut text = include_str!("operator.md").trim().to_owned();
    text.push_str("\n\n### 本次可查找 skill 的目录（按优先级排序）\n");
    let mut seen = Vec::new();
    for root in roots {
        if !seen.contains(&root) {
            seen.push(root);
            text.push_str(&format!("- `{}`\n", root.display()));
        }
    }
    if seen.is_empty() {
        text.push_str("当前没有可解析的 skill 目录，使用现有工具完成任务。\n");
    }
    Some(text)
}

pub(crate) fn instructions(session: &crate::SessionState) -> Option<String> {
    if session.session_kind.as_deref() != Some("operator") {
        return None;
    }
    render(
        session.session_kind.as_deref(),
        &crate::agent_pools::session_skill_roots(session),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operator_lists_ordered_unique_paths_and_task_contract() {
        let private = PathBuf::from("/execution/agent skills/v2");
        let shared = PathBuf::from("/execution/home/.opencoder/skills");
        let text = render(Some("operator"), &[private.clone(), shared, private]).unwrap();
        assert!(text.contains("每次调用聚焦一个可验证的结果"));
        assert!(text.contains("达到本次任务的完成条件后结束"));
        assert!(text.contains("读取对应的 `<name>/SKILL.md` 或 `<name>.md`"));
        assert_eq!(text.matches("/execution/agent skills/v2").count(), 1);
        assert!(text.find("/execution/agent skills/v2") < text.find("/execution/home"));
    }

    #[test]
    fn missing_roots_do_not_invent_a_home_directory() {
        let text = render(Some("operator"), &[]).unwrap();
        assert!(text.contains("当前没有可解析的 skill 目录"));
        assert!(!text.contains("~/.opencoder"));
    }

    #[test]
    fn other_execution_kinds_have_no_operator_instructions() {
        for kind in [
            None,
            Some("agent"),
            Some("maintenance"),
            Some("dag"),
            Some("brain"),
        ] {
            assert!(render(kind, &[PathBuf::from("/skills")]).is_none());
        }
    }
}
