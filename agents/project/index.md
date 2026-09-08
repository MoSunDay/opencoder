Commit: (working-tree, 基于 65c9d891ae905e7925277d29a87cd8e7957e8dad)

# project 模块

## 职责与边界

`opencoder-project` 驱动用户策展的 goal → milestone → todo；todo 可不挂里程碑，进入 backlog。生命周期为草稿 → Plan → Execute，支持重复执行与取消。项目结构由 [control](../control/index.md) 保存，运行由 [worker](../worker/index.md) 使用节点 `runtime.db` 执行；`project-<todo-id>` 固定归属节点。

项目运行与 [todos](../todos/index.md) 的自治 workflow 是不同入口。会话、消息和子任务通过 `Arc<dyn Store>` 持久化，项目记录通过 `Arc<dyn ProjectStore>` 持久化。平台 Node 固定使用 libsql；本地 Web/CLI/TUI 可选择独立 Project backend。

Plan 固定使用 plan Agent。Execute 可选择 agent/team/dag/brain；Plan 与 Agent 直驱按各自引用卡选择 Harness，包括 brain 解析到 Agent 的路径。原生执行器保存逐次模型请求/响应，Codex 保存提交输入、解析事件和 thread 溯源，不虚构其内部模型 wire。Team/DAG 保持其运行时详情与 `output_ref` 引用。

## 接收与运行身份

`ProjectService` 持有 `OnceLock<Arc<Deps>>`。`Deps` 包含 Store、ProjectStore、工作目录、客户端替身、可选 brain、admission 锁、活跃驱动注册表、归档根和持久化错误标记。

`runs.rs` 把接收与启动分开：

1. `accepted_attempt` 先按 `prun-*` ID、todo、kind 和原始请求确认回执；相同 ID 重试沿用原记录，不重新读取可变定义。
2. `reserve_attempt` 校验方案与执行器，保存请求、todo、目标/里程碑上下文、解析后的执行器和资源根。Store 在事务内互斥 Plan/Execute，并分配 `MAX(version)+1`；仅 Execute 将 todo 置为 running。
3. Worker 写入接受 journal 后调用 `drive_reserved`。已终态或已有活跃驱动时不重复启动。

`start_plan`、`start_execute` 和 `start_execute_with` 是本地入口；后者支持控制面预解析的 `ExecutorOverride`。存储失败会阻止后续接收。

## 会话与执行器

- `executor::resolve` 是执行器解析入口；Agent 使用引用名或 todo.agent，Team/DAG 使用资源引用或内联 spec，Brain 先路由为具体执行器。Brain 解析失败发生在 claim 之前；节点可直接使用 Server 提供的 override，不需要本地 brain Runtime。
- `plan_gen::client_for` 复用注入客户端或共享惰性客户端；只有实际运行原生执行器才解析原生模型凭据。
- `plan_gen::drive` 每次创建新 plan 会话；`executor/agent_drive.rs` 比较 Agent 名称、Harness 和资源内容摘要，身份一致且旧会话可加载时 resume。摘要排除资源物化路径索引，避免恢复后无意义地创建会话；原生旧摘要保持兼容。Agent、Harness 或资源版本改变时创建新会话，并携带当前方案与前次结果。
- Agent 输出通过运行前的消息 ID 集合识别，压缩造成消息数组长度变化时仍可区分新旧输出。
- `team_drive` 使用本地团队调度器，`dag_drive` 使用本地 DAG uplink；`brain_drive` 透传解析结果与 capability/plan 溯源。执行器 spec 的共享纯类型在 [store](../store/index.md)。

## 逐次留存

`trace/` 是 Plan/Agent 的归档边界，默认位于节点数据目录 `project-runs/<run-id>/`。

- `RunTrace::begin` 保存提交 prompt、Agent 名称、规范指令、资源名称/版本/文件摘要、Harness 和模型来源；记录本次会话消息和事件起点。Codex 未显式指定模型时以 `model_source=codex_config` 和空模型字段表示使用其配置；完成清单记录 Harness 与 thread ID，环境值不进入归档。`input_snapshot` 同时写入 run 行与 `input.json`。
- `RecordedClient` 保存实际 `ChatRequest::to_body()` 和收到的 `LlmEvent`，包括错误与子任务模型调用。事件分别写入带序号的内容及元数据文件；写入使用 fsync。
- `spawn_checked_event_flusher` 返回明确的落库结果。驱动等待消息/事件处理与 `RunTrace::finish` 完成后，再原子提交 run 终态及 todo 状态。
- `trace_manifest` 记录封闭的消息/事件范围、模型文件、子任务/子会话关联和交付文件。归档、事件落库或关联缺失会报错并阻止成功收敛。
- `project_artifact` 只登记工作目录内的常规文件，为每个显式交付文件保存独立副本、名称、大小与 SHA-256；下载读取副本。
- `trace/codex.rs` 给 Codex Execute 提供本次 JSON 交付清单路径，完成后通过同一文件登记函数校验、复制；Plan 不携带清单路径，避免方案固定旧运行位置。清单最多 64 KiB / 100 项，拒绝符号链接、绝对路径及越出工作区的文件。无交付可不写清单，非法声明使尝试失败；真实归档 I/O 故障继续触发持久化错误标记。

完整读取与分页由 Worker 按 run ID 提供，见 [worker](../worker/index.md)。已有历史记录缺少输入或边界时保留其原始输出，并明确标记留存不完整。

## 收敛与恢复

`finish_todo_run` 条件更新仍为 running 的记录，并在同一事务中处理 todo：Plan 成功写入方案并置 planned；Execute 成功置 done、失败置 failed、取消回 planned。已终态记录不会被后来的恢复流程覆盖。

`cancel()` 克隆并触发取消令牌，直到驱动完成输出和归档写入后才由 `forget_spawn` 移除活跃标记。`attempt_outcome` 优先判定取消并保留已有部分输出；工具步骤中的 assistant 文本不会将取消误判为成功。重复取消和 stale 扫描不会把正在收尾的驱动当作丢失。

`recover.rs` 处理 panic、丢失驱动与超过 5 分钟宽限期的 stale 记录；在会话再次使用前，`trace/recovery.rs` 封闭已持久化的消息/事件范围并标记 partial。平台节点重启后由用户显式恢复，不自动重跑。该恢复模型依赖单个运行时独占项目执行库；共享外置 ProjectStore 的多运行时部署不具备分布式驱动租约。

libsql 和 MySQL 提供原子接收/收敛实现。StarRocks 缺少所需跨表事务能力，Plan/Execute 在写入前明确拒绝；项目结构 CRUD 与历史查询仍可用。

## 代表性验证

- `crates/worker/tests/harness_matrix.rs`：无原生凭据的 Plan/Execute、连续三次恢复、Harness 切换、混合编排、取消与规划输入边界。`trace/codex.rs` 单测覆盖清单与文件安全、不变副本；真实二进制验证见 [Harness 验收](../../scripts/acceptance/harness/project.js)。

- `tests/plan_and_execute.rs` 与 `tests/replay/contracts.rs`：计划/执行、相同 ID 重试、Agent 切换、互斥接收、子任务关联、不可变交付文件、归档错误、取消后的部分输出。
- `src/service_tests.rs`：重复取消时保持驱动活跃、panic/stale 条件收敛；`plan_gen` 单测覆盖压缩后的输出识别。
- `tests/executor_team_dag_brain.rs`：Team/DAG/Brain 路由和节点预解析覆盖。
- `crates/worker/tests/project_replay.rs`：27 次运行、历史分页、大字段、失败尝试唯一输入、节点归属与离线错误。
- [双节点浏览器验收](../../scripts/acceptance/project/README.md)：创建层级、当前 Agent 版本、失败/取消/重启、完整索引回读与 30 分钟观察。
