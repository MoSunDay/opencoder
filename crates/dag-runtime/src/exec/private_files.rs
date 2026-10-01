use std::path::Path;

pub(super) fn prompt(prompt: String, root: Option<&Path>) -> String {
    let Some(root) = root else { return prompt };
    format!("{prompt}\n\n执行器私有任务文件目录：{}。只把其中的文件路径传给任务工具；不得将凭证内容读入模型上下文、prompt、日志或最终结果。任务目录只读，结果和临时文件写入当前步骤工作目录。", root.display())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_prompt_only_includes_guest_path() {
        let text = prompt(
            "run task".into(),
            Some(Path::new(opencoder_core::fleet::private_files::GUEST_ROOT)),
        );
        assert!(text.contains(opencoder_core::fleet::private_files::GUEST_ROOT));
        assert_eq!(prompt("task".into(), None), "task");
    }
}
