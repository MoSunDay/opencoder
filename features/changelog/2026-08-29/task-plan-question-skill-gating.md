Commit: 7687b5f581254ee6d826d8644789e7d498e761ba

# question 工具由规划技能解锁

`question` 对 act 与 sandbox 使用相同的 latent 门控：只有激活的规划技能正文前 500 字符包含工具名时才可见。sandbox 的工具 allowlist 保留调用资格，系统提示词不再常驻注入澄清协议；用法由规划技能正文提供。

内置技能只携带有实现支撑的资源。首次安装按内置资源清单写入；升级更新内置文件前备份用户修改，清单之外的用户资源保持原样。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 规划正文在前 500 字符内解锁 question | `seeded_task_plan_body_unlocks_question_in_prefix_window` | `crates/core/tests/skill_contract/planning.rs` |
| 规划技能携带完整澄清用法与准确资源清单 | `seeded_task_plan_skill_requires_question_tool_guidance`、`seeded_task_plan_skill_requires_launch_closure_contract` | `crates/core/tests/skill_contract/planning.rs` |
| 更新内置文件前备份，用户资源保持原样 | `seed_builtin_skills_backs_up_then_overwrites_user_edits` | `crates/core/tests/skill_contract/seeding.rs` |

原迭代全量回归为 3345 passed / 0 failed，clippy 零警告。当前回归结果见 [核心仓库范围](../2026-10-01/core-platform-scope.md)。
