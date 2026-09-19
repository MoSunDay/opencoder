Commit: c869027bad052c7ce91bdd0818f7be149b2b66dc

# Operator/Agent 会话提交受理超时

## Context

节点创建执行前会复制 Agent 资源快照。共享 NFS 资源池首次快照可能超过原有的
15 秒控制面 RPC 窗口，节点实际已继续受理时，operator 和 agent 提交却收到
`504 node request timed out`。

## Change Summary

控制面将 `NodeOperation::Create` 的受理窗口延长到 60 秒；普通节点查询和控制
调用仍保持 15 秒快速失败。

## Impact Surface

`POST /api/sessions`（operator/agent）以及通用执行创建入口可以等待节点完成首次
资源快照，减少误报超时。执行 ID 的幂等和节点持久化协议不变。

## Notes / Compatibility

超出 60 秒仍按原契约返回 504，并要求使用同一 execution ID 重试；不要为重试生成
新的 ID。

## Related Docs

- [control 模块](../../../agents/control/index.md)
- [Agent 调度平台](../../agent-platform/index.md)
