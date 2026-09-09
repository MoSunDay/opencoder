Commit: 2491657d33c384dddcabf4d12ab4cd8822ccaf81


# control 模块

`opencoder-control` 是平台控制面，独立于本地 session/runtime。由 [server](../server/index.md) 启动，提供 Web、全局定义、节点连接、大脑和调度。

## 边界与数据

- `bootstrap` 只打开新 `control.db` 与 `definitions.db`。前者持久化节点、五字段执行索引和团队/DAG/能力绑定及私有 Harness 配置；后者保存大脑及项目定义。平台不连接旧 MySQL 项目库。
- `core::fleet` 定义协议与纯调度函数。Server 不依赖 session、worker、team、project runtime 或 Python VM。
- `transport::Hub` 的连接、负载、待确认 RPC 与容量预留仅在内存；执行明细从拥有 ID 的 Node 读取，不缓存入库。

## 主流程

1. Node 经 Bearer 认证的 WebSocket 注册、上报快照及索引；协议必须与 Server 相同（当前 v7）。不匹配时在保存注册、索引同步及调度之前拒绝，错误包含双方协议及同步升级要求；代际和序号约束连接更新，索引归属不可改写。
2. 创建时在 placement 锁内解析全局定义与受管 Codex 配置，优先按 loops/CPU 选择有容量的节点；全部满载时按 pending 数选择可接受节点。持久化归属并预留后释放锁、发 RPC，私有 `Assignment.codex` 和 `Assignment.runtime` 携带默认及命名 Harness、Runner 的配置快照。
3. Node 接受后释放预留；同 ID 请求转发原节点比较原始输入；超时保留归属，不重新分配。
4. 明细、控制、SSE 和分页产物均通过 ID 路由。旧 Chat/DAG/TODO/Project API 由 `api/compat` 等适配。

`api/settings` 的 Harness GET/PUT 复用 FleetStore 的 `harness/codex` 定义，校验参数并递增 revision；管理 env 不进入 NFS 或公开执行明细。节点调度 PUT 转发 `configure_scheduling` maintenance RPC，实际持久化和调度由 Node 负责。

`api/brain` 在未显式指定计划且能力库为空时直接构造默认 act 目标；有能力时仍经规划和路由，缺少目标绑定时使用 act。存储、规划和节点错误直接返回，显式计划不会跳过查找。`brain_dispatch` 回执以可空的 plan/capability ID 表示默认路径，保留旧字符串回执兼容；重试先从原 Node 确认接受回执，不因能力库变化重做决策。执行索引的状态可随进度变化，执行 ID、节点归属和决策保持不变。

`bootstrap::BrainClient` 按调用类型解析聊天/向量端点。规划请求未显式设置推理强度时，继承 Server 的 `Config.reasoning_effort`；请求本身的设置优先。Server 独立读取其工作目录配置，不依赖 Node 进程的用户配置。

普通团队与工作流只分配一个 Node。跨节点 PeerCall 仅允许运行中 system 执行的所属协调节点联系维护 agent。大脑预览不派发，dispatch 解析能力的执行目标并沿用稳定请求 ID。项目 `project-<todo-id>` 保持 Plan → Act 节点归属。项目专用路由和通用执行控制共用 `executions::dispatch_command`：每次新的 Plan/Execute 解析当前 Server 草稿、TODO 绑定与项目结构，覆盖客户端自带快照，再交由原节点执行接收检查。

共享源文件仅复用 Web 的鉴权、静态资源和全局资源管理处理器；平台入口不运行旧 Web 节点任务队列。控制面 `/api` 全面子功能的 e2e 在 `crates/control/tests/e2e/`（真实 build_app + 脚本化 WS 节点 + SHARE_GATE 串行共享目录，161 用例；projects 存储可经 `new_state_with_projects` 注入以测 store 故障分支；基建旋钮见 `support/node.rs`）。执行实现见 [worker](../worker/index.md)，业务规则见 [Agent 平台](../../features/agent-platform/index.md)。

## 项目运行索引

项目提交可携带独立 `run_id`，回执包含该运行 ID 与原节点；重试先向原节点确认接收，再读取可变定义或执行 brain 解析。首次 Execute 因缺方案被拒后，后续 Plan 可重新建立尚未被 Node 接收的根记录；只有明确的 execution not found 才走该路径。

`GET /api/project/todos/:id/runs?before_version=...` 返回有界历史页。`GET /api/executions/:run-id`、`/messages`、`/events-page?after=...`、`/detail-field`、事件载荷和 `/artifact` 都向索引所属 Node 查询。全局索引不保存这些运行内容。

`resource_scope.rs` 为 `/api/agents*` 绑定当前 Server 配置的资源根，使发布 Agent/资源与 NFS 导出使用同一目录；不同 Server 的请求作用域隔离。验证入口为 `tests/resource_root.rs` 与 [Worker 的回放契约](../../crates/worker/tests/project_replay.rs)。

`api/settings/registered` 通过现有定义存储的 `codex_profile` / `runner` 命名空间保存注册配置并递增 revision；受理时固定完整快照，拒绝超出通道预算的配置。管理 API 为 `/api/harnesses/codex/profiles` 与 `/api/runners`，默认 `harness/codex` 独立保留。注册 Runner 通过 DAG step 使用，执行与产物仍归原 Node。协议见 [注册 Runner](../../docs/registered-runners.md)。
