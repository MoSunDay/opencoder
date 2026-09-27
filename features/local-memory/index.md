Commit: 40033c02

# 本地仓库记忆

`/config` 的 `local-memory` 默认关闭。打开后，成功完成的主任务才会继续运行内置 `repo-local-memory`，且仅限 act 模式的主任务（plan 只读问答、command 一次性任务、workflow 调度与 subagent 均不触发）；按技能规则更新仓库记忆，更新完成后任务才进入结束状态。判定以任务实际运行的 Agent 为准——plan 会话中提交的 `/act 任务` 复合输入同样会执行维护。

记忆维护使用主任务上下文的副本，具有独立会话与消息历史。它产生的内容不追加到主会话；用量计入该任务。维护过程以可折叠的 `memory` 子代理块实时回显在 TUI 对话区（复用 subagent 路由）：块的头部显示维护指令，维护期间执行的每个工具调用、输出与文本都流入块内，结束时折叠块以摘要（成功）或失败原因收口；Web SSE 与 headless 输出经同一事件流获得相同回显。中断或失败的任务不触发维护；技能缺失或维护执行失败会直接报告错误。

TUI 暂不提供 Agent 命令入口。见 [会话运行时](../../agents/session/index.md)、[配置](../../agents/core/index.md)、[TUI](../../agents/tui/index.md)。
