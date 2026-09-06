Commit: (working-tree, 基于 c1a1b2e78e1ccd4a3cc2ac6dc408a76d30bf46e6)

# Agent 调度平台

平台用户通过 Web 管理节点、资源、团队、工作流、项目及大脑。`opencoder-server` 管理调度，`opencoder-agent` 执行；CLI/TUI 独立保留。

## 规则

- 普通 agent、team、DAG、TODO 和项目 Plan → Act 分配后固定一个节点；该执行的全部子 agent 与运行数据在节点闭环。
- Server 的运行索引只有创建时间、ID、调度节点、状态。查看输入、消息、计划、结果、事件或产物时，由 Server 按 ID 查询所属节点；节点离线明确报错。
- 调度优先活跃 agent loops / CPU 最低的合格节点，考虑待确认分配及容量；空闲会话不占 loop。指定节点不能自动改派。
- 稳定创建 ID 支持幂等重试；同 ID 不同输入返回冲突。网络超时保留原节点归属，断线后已接受执行继续。
- 重启未完成执行显示 interrupted，由用户显式在原节点恢复；不会自动重跑或修复。Python 取消和超时等待物理执行停止后才完成收尾。
- 项目每次显式 Plan 使用当前全局草稿，所有控制入口行为一致。忙碌、容量或预检拒绝不会改写上次记录。
- NFS 只共享 agent 资源。执行固定实际版本文件，后续发布或删除不改变已接受的执行；配置的共享目录不可用、非只读 NFS、资源引用缺失均明确失败；共享目录离线时，已接收执行仍使用本地快照继续或恢复，新执行拒绝接收。
- 普通团队成员由 agent 与职责定义，统一在一个节点执行。内置 system 团队是跨节点例外，由各节点维护 agent 组成，只响应用户维护指令。
- 大脑能力绑定 agent/team/DAG/TODO，预览只查看路由，直接调度按稳定 request_id 启动。
- 新平台使用独立存储，不迁移或删除旧 daemon/CLI 历史。

## 入口与状态

Web 的节点页支持负载与显式维护，全部执行页支持统一创建、查询、取消、恢复；团队页配置职责，大脑页配置能力绑定。项目、会话、DAG、TODO 页面继续可用。

主要状态为 pending、running、idle、interrupted、done、error、cancelled。会话一次回答后为 idle；项目可继续 Plan/Act；工作流完成后进入终态。节点持久化错误会使节点不可调度。

部署、API 与已知运行时边界见 [部署说明](../../docs/agent-platform.md)。逻辑结构见 [control](../../agents/control/index.md)、[worker](../../agents/worker/index.md)、[node](../../agents/node/index.md)。
