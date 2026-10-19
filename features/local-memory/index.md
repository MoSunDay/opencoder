Commit: 9a8213e3dba80353fefcf7d20aa57bf7461f27c8

# 本地仓库记忆

`/config` 的 `local-memory` 默认关闭。打开后，成功完成的主任务会继续运行内置记忆维护流程，更新受影响的仓库说明；更新完成后任务才进入结束状态。

记忆维护使用主任务上下文的副本，具有独立会话与消息历史。它产生的内容不追加到主会话；用量计入该任务。中断或失败的任务不触发维护；技能缺失或维护执行失败会直接报告错误。

只在原生 OpenCoder 的 act 主会话完成任务后维护，计划模式、工作流和子会话不触发。外部 Codex 自己管理任务生命周期及模型凭据，不再额外启动原生 Act 记忆会话。维护会话使用宿主命令工具：Linux/macOS 为 `bash`，Windows 为 `powershell`。见 [会话运行时](../../agents/session/index.md)、[配置](../../agents/core/index.md)、[TUI](../../agents/tui/index.md)。
