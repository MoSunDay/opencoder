Commit: b465f440381bd009dc9bd3a8192ad88eab44cede

# Agent 调度平台 — Server 调度、Node 执行、Web/CLI 管理

## 关键路径

- crates/control/src/api/executions/mod.rs — 稳定 ID 幂等受理与 select_queue_node
- crates/control/src/api/settings/ — harness/runner 私有定义库
- crates/control/src/admission.rs — Open/Frozen 受理开关
- crates/web/src/api_nodes*.rs — 节点注册、负载与维护接口
- crates/web/src/api_project*.rs — 项目 API 与运行回放
- crates/worker/src/operations/launch.rs — 受理快照（harness + 资源版本）
- crates/worker/src/operations/queue/mod.rs — 持久化 pending 队列与派发
- crates/node/src/uplink.rs — WebSocket 注册/心跳/执行 RPC
- crates/node/src/runner.rs — 注册 Runner 执行
- crates/server/src/main.rs — opencode-server 二进制
- crates/agent/src/main.rs — opencode-agent 二进制
- crates/web/spa/src/fleet/nodes.jsx — 节点页与调度配置
- crates/web/spa/src/fleet/executions.jsx — 全部执行页
- crates/web/spa/src/fleet/detail/ — 执行详情与回放
- docs/agent-platform.md — 部署、API 与运行时边界
- docs/registered-runners.md — 注册 Runner 约定
- scripts/acceptance/business/README.md — 独立副本业务验收

## 边界

- 执行分配后固定节点，子执行与数据节点闭环
- Server 索引仅创建时间/ID/类型/节点/状态，明细按 ID 回查节点
- 节点离线时明细查询明确报错
- NFS 只共享 agent 资源，执行固定受理时版本快照
- 新平台独立存储，不迁移旧 daemon/CLI 历史

## 相关

- [agents/control](../../agents/control/index.md)
- [agents/worker](../../agents/worker/index.md)
- [agents/node](../../agents/node/index.md)
- [项目管理](../../agents/project/index.md)
- [Agent Harness](../harness/index.md)
