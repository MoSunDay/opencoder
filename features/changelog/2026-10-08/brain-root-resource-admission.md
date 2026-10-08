# 大脑根任务按实际依赖准入

Schema 7 根任务只用已冻结的计划、能力与执行证据进行有限决策，不再复制整个 Agent 资源库。准入持久化独立的空资源目录；子执行仍按各自的资源规则准入，既有运行重放沿用已保存的目录和回执。

线上日志记录到根任务复制 383 项资源用了 25～44 秒。真实 Worker 测试 `finite_root_decides_without_agent_files_but_agent_children_still_require_them` 验证根任务在 Agent 源不可用时仍完成一次决策、重放不增加决策，Agent 子执行仍拒绝缺失的资源源。真实模型和 runc 用例 `root-resources` 检查根任务受理不超过 30 秒、独立资源目录为空，以及子执行实际诊断通过后才完成。

人工引导验收清理只释放仍在运行的子任务，避免尚未创建的执行详情 404 覆盖原始启动超时。验证映射：`test_admission_timeout_is_preserved_when_children_are_not_created`、`test_cleanup_does_not_touch_terminal_child_workspaces`，以及真实工具等待与释放测试。按用户要求，仅验证这些改动涉及的资源准入、有限决策和验收脚本。

真实人工引导用例 `live-steering` 使用 `plan` 负责 Team 规划，保留 `act` 执行实际工具诊断、后续 `plan` 成员检查新标记。避免规划回答触发 Act 的任务后记忆维护而挤占工具启动时间；工具等待、实际引导回执、最终结果检查与超时限制保持原有要求。
