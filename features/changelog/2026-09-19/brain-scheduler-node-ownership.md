Commit: 586e56014aaa9d2ec59047a24a8cb5ca644d1249

# Brain v3 调度归属收尾

## Context

事件驱动调度 v3 已从控制面持有完整投影的实现收敛为节点归属的运行时。控制面需要继续提供能力目录和统一执行入口，同时避免在 Control 与 Worker 保存两份可竞争的 generation 和轮次状态。

## Change Summary

- Worker 根节点持有 v3 scheduler projection、generation、模型上下文及下一轮唤醒；节点重启恢复未确认事件、未完成创建和取消请求。
- Control 负责归一化 Agent、Team、DAG、TODO 能力描述，按真实 target 创建普通子执行，并处理中继回执、终态事件和迟到通知。
- 根节点在持久化 scheduler context 后重新排队 idle root，确保 node-local model 使用最新上下文；失败终态立即取消同轮兄弟操作。
- 终态 outbox 只发送 Done、Error、Cancelled；取消兄弟的迟到终态可完成审计写入，但不改变已终态脑状态。

## Impact Surface

- v3 快照、轮次和事件读取通过根节点索引，子执行详情仍按 `execution_id` 查询。
- v2 运行与旧数据保持只读兼容；未增加环境变量或删除任何数据。
- Control/Worker/Brain 的纯函数、Store、outbox 和节点集成测试已通过，Clippy `-D warnings` 通过。

## Related Docs

- [agents/control](../../../agents/control/index.md)
- [agents/worker](../../../agents/worker/index.md)
- [agents/brain](../../../agents/brain/index.md)
- [brain orchestration](../../../docs/brain-orchestration.md)
