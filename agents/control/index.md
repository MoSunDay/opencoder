Commit: b465f440381bd009dc9bd3a8192ad88eab44cede

# control 模块

平台控制面：节点接入、全局定义、执行索引与调度。

## 关键路径

- `crates/control/src/bootstrap.rs` — 仅开 control.db + definitions.db；BrainClient
- `crates/control/src/transport/hub.rs` — Hub：协议 v7 校验；连接/RPC/预留仅内存
- `crates/control/src/api/executions/mod.rs` — `submit_inner` 提交选点：placement 锁内解析定义并预留；按 ID 路由明细/控制/SSE/产物
- `crates/control/src/api/catalog.rs` — nodes/maintain/teams/dag_defs/resolve 维护与定义解析
- `crates/control/src/api/compat/` — 旧 Chat/DAG/TODO/Project 兼容路由
- `crates/control/src/api/settings/` — harness/codex 定义；registered 管 profile/runner
- `crates/control/src/routes.rs` — /api/harnesses/codex/profiles、/api/runners
- `crates/control/src/resource_scope.rs` — /api/agents* 绑定 Server 资源根
- `crates/core/src/fleet/protocol.rs` — PROTOCOL_VERSION=7；ExecutionIndex 五字段
- `crates/control/tests/e2e/` — e2e：真实 build_app + 脚本化 WS 节点
- `crates/control/tests/resource_root.rs` — 资源根隔离验证

## 边界

- server 二进制不依赖 session/worker/team/project runtime。
- 执行明细向归属 Node 实时查询，全局索引不存运行内容。
- system 团队执行已退役；跨节点维护走 POST /api/nodes/:id/maintenance。

## 相关

- [server](../server/index.md) 启动方；[worker](../worker/index.md) 执行面
- [Agent 平台](../../features/agent-platform/index.md)、[注册 Runner](../../docs/registered-runners.md)
- 回放契约 [project_replay.rs](../../crates/worker/tests/project_replay.rs)
