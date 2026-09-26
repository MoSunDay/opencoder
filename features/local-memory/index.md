Commit: c60e2162be48102badf53d8b97e7cfa030b59605

# 本地仓库记忆

`/config` 的 `local-memory` 默认关闭。打开后，成功完成的主任务会继续运行内置 `repo-local-memory`，按技能规则更新仓库记忆；更新完成后任务才进入结束状态。

记忆维护使用主任务上下文的副本，具有独立会话与消息历史。它产生的内容不追加到主会话；用量计入该任务。中断或失败的任务不触发维护；技能缺失或维护执行失败会直接报告错误。

TUI 暂不提供 Agent 命令入口。见 [会话运行时](../../agents/session/index.md)、[配置](../../agents/core/index.md)、[TUI](../../agents/tui/index.md)。
