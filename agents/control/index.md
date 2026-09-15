Commit: 3c1222a5e61536ec96914a7edb40d246bc6665e6

# control 模块

平台控制面：节点接入、全局定义、执行索引与调度。

## 关键路径

- `crates/control/src/bootstrap.rs` — 仅开 control.db + definitions.db；BrainClient；兼容模式管理 NFS，版本模式转发到独立资源服务
- `crates/control/src/transport/hub.rs` — Hub：协议 v9 校验、连接与在途 RPC；按 Host 交接编号隔离旧报告，完整索引同步后标记就绪
- `crates/control/src/api/executions/mod.rs` — 持久请求去重与归属路由；`submit.rs` 以跨进程锁保护选点/冻结 assignment，事务保存回执和派发记录，`release/outbox.rs` 恢复未确认派发
- `crates/control/src/api/brain_runs/` — 能力/不可变计划/运行 API；按根串行授权派发与控制，资源占用和来源回执确认。
- `crates/control/src/api/catalog.rs` — 节点列表、注册删除、维护，以及 teams/dag_defs/resolve 定义解析
- `crates/control/src/api/compat/` — 旧 Chat/DAG/TODO/Project 兼容路由
- `crates/control/src/api/compat/todo_review.rs` — Review 与任意节点重跑转发到原归属 Node；context-preview 复用 TODO 上下文纯函数。
- `crates/control/src/api/settings/` — harness/codex 定义；registered 管命名 Codex profile
- `crates/control/src/routes.rs` — /api/harnesses/codex/profiles 与 TODO files/validate/version 路由；目录 API 复用 Web 实现
- `crates/control/src/resource_scope.rs` — /api/agents* 绑定 Server 资源根；/api/dag/wasm* 复用共享中间件 `api_dag_wasm_nfs::configured_dag_wasm`
- `crates/control/src/role_gate.rs` — 角色权限矩阵纯函数；layer 顺序 auth → role_gate → resource_scope；/api/dag/wasm* 非 admin 只读
- `crates/control/src/api/users.rs` — /api/me、/api/users CRUD；token 一次性明文、自删/末位 admin 保护
- `crates/core/src/fleet/protocol.rs` — PROTOCOL_VERSION=9；ExecutionIndex 五字段
- `crates/control/tests/e2e/` — e2e：真实 build_app + 脚本化 WS 节点
- `crates/control/tests/resource_root.rs` — 资源根隔离验证
- `crates/control/src/api/brain_playbook_dispatch.rs` — 剧本平台派发：批次按依赖层背靠背建 execution（id `{prefix}-pbk-{request_id}-{step}`，key 原样不截断、request_id ≤26 字符否则 400；`PlaybookGate` 同 request_id 异派发内容 409、容量满 503）、`fleet.definition("capability_target", id)` 解析 Brain 目标（内联 `PlaybookRoute` 压过绑定；占位步骤空 situation 400）、`trigger_scan` 消息相似度触发（入站 + 全部 match_text 单批 embed）；路由 `GET/POST /api/brain/playbooks*`。
## 边界

- server 二进制不依赖 session/worker/team/project runtime。
- 执行明细向归属 Node 实时查询，全局索引不存运行内容。Brain 根明细同样属于 Node，计划版本与跨运行资源占用属于 control。
- TODO Review/重跑复用协议 v9 的执行 Command；新增接口仍受既有角色权限矩阵约束。Agent 列表的 primary 标记来自实际注册表解析。
- NFS 状态读取等待导出生命周期锁；启动后的状态检查与导出操作串行，锁竞争不会被解释为导出停止。
- Brain 管理动作派发前探测目标节点资源摘要，原生受理复核快照；不确定受理重发同 ID。
- system 团队执行已退役；跨节点维护走 POST /api/nodes/:id/maintenance。
- team 定义成员=agent 名（唯一、captain ∈ members）；resolve 时经 `GET /api/brain/agents` 同源聚合把成员能力 summary 固化进 pinned definition，库存定义不落 capabilities；成员名/captain 在 validate 时就地 trim 归一（与 bind 侧对称，padded 提交不再固化空快照）。
- 认证开启时 seed token 恒等 admin；换启动 token 重启会把表内 `admin` 行 digest 重指新 token（轮换即吊销旧 seed 凭证）。非 admin 仅读 + operator 提交/命令（operator 为宿主机直跑通道）；无 Identity 视为 admin（本地模式）。
- 节点删除只移除 FleetStore 的 `fleet_nodes` 注册行；在线连接返回 409，离线注册删除后执行索引和节点本地任务数据仍由各自生命周期管理。

## 多版本协议

- `release/` 分离本实例退役与管理员冻结；响应跟踪覆盖完整 body，保留长度和 trailers。SSE 发送最后游标与重连通知，普通请求无发布强杀期限。
- Brain、Playbook、Project 长操作使用本机文件锁，数据库事务不跨网络或模型调用。相同 ID/内容返回原回执，异内容返回冲突。
- Project 首次派发回执按 run_id 区分；新尝试仅可替换明确拒绝的派发，执行归属保持原节点，未知结果禁止重派。
- 发布状态、资源管理由 `release/proxy.rs` 连接 Host/独立 NFS；整机容量属于 Host 持久账本。
- `release/signals.rs` 在监听前注册 USR2/USR1，使用现有凭证向本机 Host 请求发布/回滚；`proxy.rs` 声明 `signal_protocol`。Server 不持有部署作业，信号接受与部署成功由不同回执区分。

## 相关

- [brain](../brain/index.md) 本体调度与回执契约
- [server](../server/index.md) 启动方；[worker](../worker/index.md) 执行面
- [Agent 平台](../../features/agent-platform/index.md)、[TODO 工作流](../../features/todos/index.md)
- 回放契约 [project_replay.rs](../../crates/worker/tests/project_replay.rs)
