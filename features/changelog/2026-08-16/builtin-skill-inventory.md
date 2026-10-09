Commit: 7687b5f581254ee6d826d8644789e7d498e761ba

# 内置技能清单与用户资源

内置技能只包含通用工作流与仓库记忆能力。首次安装检查完整技能集合，防止缺失技能或额外写入资源。内置文件更新时先备份用户修改；不属于内置清单的用户资源原样保留。外部 CLI 由用户自行注册，按 Session 的工具权限注入。

当前实现与技能清单见[核心逻辑索引](../../../agents/core/index.md)。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 首次安装的技能集合准确、无多余资源 | `seed_in_writes_all_packs_on_fresh_dir` | `crates/core/tests/skill_contract/seeding.rs` |
| 内置文件备份更新，自定义用户资源保持原样 | `seed_builtin_skills_backs_up_then_overwrites_user_edits` | `crates/core/tests/skill_contract/seeding.rs` |
| CLI 注册按子 Agent 名称与工具权限注入 | `cli_injected_only_into_explore_subagent_by_name`、`mcp_tools_hidden_from_workflow_agent` | `crates/session/src/runner/llm_call.rs` |
