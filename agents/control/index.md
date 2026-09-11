Commit: bf757d2e8688496f8c1be8fc8926dbd9fb2e3c21

# control 模块

平台控制面：节点接入、全局定义、执行索引与调度。

## 关键路径

- `crates/control/src/bootstrap.rs` — 仅开 control.db + definitions.db；BrainClient
- `crates/control/src/transport/hub.rs` — Hub：协议 v8 校验；连接/RPC/预留仅内存
- `crates/control/src/api/executions/mod.rs` — `submit_inner` 提交选点：placement 锁内解析定义并预留；按 ID 路由明细/控制/SSE/产物
- `crates/control/src/api/catalog.rs` — nodes/maintain/teams/dag_defs/resolve 维护与定义解析
- `crates/control/src/api/compat/` — 旧 Chat/DAG/TODO/Project 兼容路由
- `crates/control/src/api/settings/` — harness/codex 定义；registered 管 profile/runner
- `crates/control/src/routes.rs` — /api/harnesses/codex/profiles、/api/runners
- `crates/control/src/resource_scope.rs` — /api/agents* 绑定 Server 资源根；/api/dag/wasm* 复用共享中间件 `api_dag_wasm_nfs::configured_dag_wasm`
- `crates/control/src/role_gate.rs` — 角色权限矩阵纯函数；layer 顺序 auth → role_gate → resource_scope；/api/dag/wasm* 非 admin 只读
- `crates/control/src/api/users.rs` — /api/me、/api/users CRUD；token 一次性明文、自删/末位 admin 保护
- `crates/core/src/fleet/protocol.rs` — PROTOCOL_VERSION=8；ExecutionIndex 五字段
- `crates/control/tests/e2e/` — e2e：真实 build_app + 脚本化 WS 节点
- `crates/control/tests/resource_root.rs` — 资源根隔离验证

- `src/api/brain_playbook_dispatch.rs` — 剧本平台派发：批次按依赖层背靠背建 execution（id `{prefix}-pbk-{request_id}-{step}`）、`fleet.definition("capability_target", id)` 解析 Brain 目标、`trigger_scan` 消息相似度触发；路由 `GET/POST /api/brain/playbooks*`。
## 边界

- server 二进制不依赖 session/worker/team/project runtime。
- 执行明细向归属 Node 实时查询，全局索引不存运行内容。
- system 团队执行已退役；跨节点维护走 POST /api/nodes/:id/maintenance。
- team 定义成员=agent 名（唯一、captain ∈ members）；resolve 时经 `GET /api/brain/agents` 同源聚合把成员能力 summary 固化进 pinned definition，库存定义不落 capabilities；成员名/captain 在 validate 时就地 trim 归一（与 bind 侧对称，padded 提交不再固化空快照）。
- 认证开启时 seed token 恒等 admin；换启动 token 重启会把表内 `admin` 行 digest 重指新 token（轮换即吊销旧 seed 凭证）。非 admin 仅读 + operator 提交/命令（operator 为宿主机直跑通道）；无 Identity 视为 admin（本地模式）。

## 相关

- [server](../server/index.md) 启动方；[worker](../worker/index.md) 执行面
- [Agent 平台](../../features/agent-platform/index.md)、[注册 Runner](../../docs/registered-runners.md)
- 回放契约 [project_replay.rs](../../crates/worker/tests/project_replay.rs)
