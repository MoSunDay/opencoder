Commit: c799d510

# 本地仓库记忆

`/config` 的 `local-memory` 默认关闭。打开后，仅成功完成的 act 模式主任务会继续运行内置 `repo-local-memory`（plan 只读问答、command 一次性任务、workflow 调度与 subagent 均不触发）；按技能规则更新仓库记忆，任务在维护完成后才进入结束状态。

记忆维护在独立会话中运行于任务上下文的副本之上：过程以可折叠块的形式在对话区实时回显，维护用量计入任务的 tok 成本，执行时长在块收口时展示。中断或失败的任务从不触发维护；技能缺失或维护执行失败会直接报告错误。

TUI 暂不提供 Agent 命令入口。见 [会话运行时](../../agents/session/index.md)、[配置](../../agents/core/index.md)、[TUI](../../agents/tui/index.md)。
