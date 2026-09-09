Commit: 2491657d33c384dddcabf4d12ab4cd8822ccaf81


# worker 模块

## Harness 调度

`operations/create` 按各 Agent 的 Harness 预检：原生执行器检查模型配置，Codex 检查受管配置或节点 PATH 中的可执行入口；TODO 预检同时覆盖 workflow 父 Agent。`workloads/agent` 在固定资源作用域内读取本次 `input.harness` / `input.envs` / 模型并初始化会话，后续运行与续聊交给共享 [session](../session/index.md)。Team、DAG、TODO、Project 创建的 Agent Session 同样使用各自引用卡默认值。

Project 预检区分资源定义与实际调用：所有引用 Agent 必须存在，Plan 只检查 plan 的执行凭据，Execute 检查实际执行器；后续命令使用 `next_action`，不能沿用首次接收的 action。`tests/harness_matrix.rs` 覆盖 Project、Team、DAG、TODO、原生父会话的 Codex 子任务、双向混合及四类取消回收。

`Assignment.codex` 与 `Assignment.runtime` 在接收时进入私有 `Config.agent.codex/runtime`，受管 Codex 拒绝按次 model/env 覆盖。`harness::scope` 使重新读取配置的内部驱动保持已接受的参数，fork 继承父 Assignment；工作负载 Future 和完整队列快照使用堆分配，避免深层编排作用域放大线程栈。

节点保留恢复所需环境，`operations/query` 对公开输入和大字段读取隐藏环境值，会话详情只单列 Harness 名称。Server → Node → Codex 二进制、消息回放、幂等提交和续聊由 `tests/harness_codex.rs` 验证。用户入口见 [Agent Harness](../../features/harness/index.md)。

`opencoder-worker` 持有节点执行数据和工作负载适配器，实现 [node](../node/index.md) 的 `NodeService`。由 [agent](../agent/index.md) 二进制构造。

## 所有权

`WorkerOptions` 指定独立数据目录、工作目录、顶层容量和 DAG 能力。节点 ID 持久化、目录加锁。`runtime.db` 保存 session/workflow/project 明细，`<kind>/<id>/execution.json` 保存原始请求、定义快照与状态日志（兼容旧 `executions/` 布局），资源、团队和产物目录均在 Node。

`operations/create` 在 admission 锁内去重、预检、固定资源，并 fsync pending 接受记录；`operations/queue` 按单调序号与 FIFO/LIFO 选择等待任务，取得容量后交给 `launch`。Plan 快照在忙碌、类型、目标和资源检查通过后写入接收记录，容量不足进入队列；拒绝请求不改变 journal。新快照预检失败会清理未接收副本，允许同 ID 重试读取最新资源。运行和控制不依赖 WebSocket 存活。重启时运行中记录变 interrupted，显式 resume 才运行；带有效队列快照且无停止意图的 pending 保留。落盘故障让节点不可调度。

`runtime/scheduling` 在 Node 数据目录持久化 `scheduling.json`，记录最大顶层并发与队列顺序，保存值优先于启动默认值。`try_slot` 在 admission 锁内执行动态上限检查；降低上限不取消现有 permit。调度器随 Node 停止退出，冻结期间不派发。空闲会话 prompt/compact/handoff 超额时原子保存待执行命令；相同待调度输入幂等，不同输入冲突。Project 队列恢复预约注册，运行索引和重试回执保持 pending。

## 执行适配

- agent：复用 [web](../web/index.md) 本地 session API、drain 和消息事件持久化；HTTP router 在进程内调用，Node 不开放入站 HTTP。
- 普通 team：自定义 `TeamDispatcher` 为每个成员创建本地会话；职责与定义固定。
- system team：协调记录本地保存，远端成员通过 PeerBridge 调用各节点维护 agent，维护明细仍属远端。
- TODO：[todos](../todos/index.md) Runtime 的父会话与子执行都使用本地 Store。
- DAG：[dag-runtime](../dag-runtime/index.md) 的 uplink 接到本地持久化，产物经命令分页读取；resume 跳过已成功落盘的检查点。
- project：[project](../project/index.md) Runtime 使用节点库，项目结构来自 Server 快照；`operations/project_admission/` 为每次 Plan/Execute 接收独立 run ID，`workloads/project.rs` 仅驱动已接收尝试并跟踪终态。

`resources` 对新执行校验显式资源目录为只读 NFS，再复制当前资源文件形成不可变快照；失败拷贝清理 staging。普通会话继续/恢复使用本地快照；项目每次新的 Plan/Execute 固定当前资源，相同 run ID 重试沿用原回执。NFS 不可用时拒绝新执行。`core::agent::scope` 和 session runner 传播任务局部资源根，避免并发执行串用版本。`session::loop_registry` 提供真实活跃 loop；顶层容量与 loop 计数分开。

维护工具通过 session extension 注册，只在维护会话中暴露真实状态、配置和任务控制接口；注册与心跳不会自动发起修复。业务规则见 [Agent 平台](../../features/agent-platform/index.md)。

## 项目回放查询

`operations/query/project/` 按 `prun-*` 查询 run、留存状态与过程清单；`project-<todo-id>` 保持根归属。输入、方案、输出、模型文件、事件载荷和已登记产物通过该 ID 路由，归属节点离线时明确失败。

消息读取限制在 `messages_after < seq <= messages_through`；零 offset 游标为排他游标，从旧尝试重定位时清零 offset，避免跳过本次首条输入或混入后续运行。历史 runs 使用 `before_version`，事件使用 `after`；大字段/载荷最多每块 64 KiB。`retention` 区分 complete、partial 与 incomplete_history。字段与产物读取校验所属运行及逻辑文件名。

契约见 `tests/project_replay.rs`；真实 NFS、节点重启及浏览器检查见 [项目验收脚本](../../scripts/acceptance/project/README.md)。

## Runner 受理与回放

Runner DAG 预检包含注册入口、安装文件校验和与命名 Codex profile。pending 配置投影读取已接受队列快照；开始后继续沿用该版本。`operations/query/runner` 返回阶段、配置版本、业务摘要、独立 verdict、报告及投递状态，隐去私有环境。DAG 的 `/messages` 读取父会话持久化转译消息，报告经受控 artifact 接口下载。

`annotate` 独立保存业务对账/投递状态，不改变执行终态。已启动的 Runner 不提供普通 resume 入口，避免外部业务重复执行；有效收据在恢复时仍须重新校验。端到端合同见 [runner_dispatch.rs](../../crates/worker/tests/runner_dispatch.rs)。
