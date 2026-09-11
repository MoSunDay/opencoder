Commit: 86567de28148c84077cdc77229bff38b5b5ab11e

# control 模块

平台控制面：节点接入、全局定义、执行索引与调度。

## 关键路径

- `crates/control/src/bootstrap.rs` — 仅开 control.db + definitions.db；BrainClient
- `crates/control/src/transport/hub.rs` — Hub：协议 v8 校验；连接/RPC/预留仅内存
- `crates/control/src/api/executions/mod.rs` — `submit_inner` 提交选点：placement 锁内解析定义并预留；按 ID 路由明细/控制/SSE/产物
- `crates/control/src/api/catalog.rs` — 节点列表、注册删除、维护，以及 teams/dag_defs/resolve 定义解析
- `crates/control/src/api/compat/` — 旧 Chat/DAG/TODO/Project 兼容路由
- `crates/control/src/api/settings/` — harness/codex 定义；registered 管 profile/runner
- `crates/control/src/routes.rs` — /api/harnesses/codex/profiles、/api/runners
- `crates/control/src/resource_scope.rs` — /api/agents* 绑定 Server 资源根
- `crates/control/src/role_gate.rs` — 角色权限矩阵纯函数；layer 顺序 auth → role_gate → resource_scope
- `crates/control/src/api/users.rs` — /api/me、/api/users CRUD；token 一次性明文、自删/末位 admin 保护
- `crates/core/src/fleet/protocol.rs` — PROTOCOL_VERSION=8；ExecutionIndex 五字段
- `crates/control/tests/e2e/` — e2e：真实 build_app + 脚本化 WS 节点
- `crates/control/tests/resource_root.rs` — 资源根隔离验证

## 边界

- server 二进制不依赖 session/worker/team/project runtime。
- 执行明细向归属 Node 实时查询，全局索引不存运行内容。
- system 团队执行已退役；跨节点维护走 POST /api/nodes/:id/maintenance。
- 认证开启时 seed token 恒等 admin；换启动 token 重启会把表内 `admin` 行 digest 重指新 token（轮换即吊销旧 seed 凭证）。非 admin 仅读 + operator 提交/命令（operator 为宿主机直跑通道）；无 Identity 视为 admin（本地模式）。
- 节点删除只移除 FleetStore 的 `fleet_nodes` 注册行；在线连接返回 409，离线注册删除后执行索引和节点本地任务数据仍由各自生命周期管理。

## 相关

- [server](../server/index.md) 启动方；[worker](../worker/index.md) 执行面
- [Agent 平台](../../features/agent-platform/index.md)、[注册 Runner](../../docs/registered-runners.md)
- 回放契约 [project_replay.rs](../../crates/worker/tests/project_replay.rs)
