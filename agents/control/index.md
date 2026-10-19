Commit: b7f67dfb539a8f1fdc58d65e8ea10ef74d8c85ce

# control 模块

平台控制面：节点接入、全局定义、执行索引与调度。

[routes.rs](../../crates/control/src/routes.rs) 的 `build_app_with_auth` 将公共 HTTP 鉴权与机器凭据分开配置；`GET /api/auth/status` 无需凭据，只返回开关。关闭时中间件注入固定 admin 身份，角色与评论接口沿用同一身份；[bootstrap.rs](../../crates/control/src/bootstrap.rs) 关闭时跳过管理员凭据写入，保留机器通道与资源代理所需凭据。

## 索引
- [brain_runs/v4/io.rs](../../crates/control/src/api/brain_runs/v4/io.rs) — 新根运行选择编译时的 IO 版本，嵌套运行继承父请求；历史根 context 不带新字段。派发重放先读取原 Assignment，claim 尚无 Assignment 时只接受与原指纹完全一致的根请求。
- [api/users.rs](../../crates/control/src/api/users.rs) — 创建用户时原子保存用户与初始 Token 摘要，成功返回 `{user,token}` 并设置 `Cache-Control: no-store`；明文只在创建响应返回。仅管理员可管理，启动管理员凭据只读，角色变更立即生效；[api/users/tokens.rs](../../crates/control/src/api/users/tokens.rs) 保留独立 Token 的管理 API。
- [api/project/details.rs](../../crates/control/src/api/project/details.rs) — 项目、专项和 TODO 详情复用已有关系投影；TODO 对话按当前关联定位同一原生执行。
- [评论 API](../../crates/control/src/api/project/comments.rs) — 控制台的 TODO 评论读取与追加；人由认证身份署名，能力由共享目录核验，执行来源复用准入能力核验。沿用项目读写权限，重试先读取既有评论，名称变更和能力删除不破坏已受理重试。
- [共享人员 API](../../crates/web/src/api_project_people.rs) — `/api/project/people` 仅向已认证用户提供用户名；三层人员写入沿用项目权限，非法人员返回 400，完整修改在 Store 事务内回滚。
- `src/bootstrap.rs` — control.db、definitions.db 与独立 ontology.db 装配；就绪检查包含 Ontology 数据库和正文根，Server 退休等待已接纳的 Ontology 写事务排空。
- `src/transport/hub.rs` — Node WS Hub（协议校验、RPC）
- [api/session.rs](../../crates/control/src/api/session.rs) — 会话执行面（`?kind=operator|agent` 泳道）；历史列表先排除内部 Agent 索引，再有界并发读取节点详情，保持索引顺序与错误行。
- [api/executions/submit.rs](../../crates/control/src/api/executions/submit.rs)、[transport/socket.rs](../../crates/control/src/transport/socket.rs) — 会话首轮提交转发所选节点，及时或迟到的接收确认发布 Server 索引；拒绝不生成占位会话。资源等待中的取消通过原派发回执持久化并去重，见 [resource_admission](../../crates/control/src/api/resource_admission/mod.rs)。
- `src/api/executions/`、`src/role_gate.rs` — 派发去重、选点冻结、回执与角色门禁；执行索引五字段协议
- [resource_admission](../../crates/control/src/api/resource_admission/mod.rs) — 通过既有 Fleet 定义保存外部资源提供方；核验冻结的 `_resource_request`，提供按执行 ID 查询、状态确认和释放接口。批次释放先检查 Server 终态；等待取消与派发共用执行锁。
- [资源提供方版本](../../crates/control/src/api/resource_admission/providers.rs) — 管理员登记不可变的命名配置；`_resource_provider: {name, version}` 在首次派发前与执行 ID 绑定，后续重试、查询、取消和释放沿用冻结配置。未指定名称的既有任务保留全局接口；配置与凭据路径不发送到节点。
- `src/api/settings/` — Harness 配置与节点调度配置（Maintenance RPC 转发）
- `src/api/catalog.rs`、`src/api/compat/`、`src/api/project.rs` — 节点/定义目录、兼容路由与 Project 中继；项目、专项、TODO、Tag 处理器复用 Web 实现并经 `src/routes.rs` 装配
- [api/project_links.rs](../../crates/control/src/api/project_links.rs)、[api/project_links/](../../crates/control/src/api/project_links/) — TODO 保存执行与能力引用；`dispatch` 以稳定执行 ID 和既有 Fleet 回执复用 Agent、Operator、Team、DAG、TODO 工作流及保存的 Brain 计划入口，重试不能改变输入或能力。`reference` 按真实准入记录核验能力，不信任任意请求标签。
- [api/executions/results/](../../crates/control/src/api/executions/results/) — `GET /api/executions/:id/result` 从所属节点即时读取有界结论，不在 Server 缓存；失败、离线与内容省略显式返回。
- `src/api/compat/dag_instances.rs`、`src/api/stream.rs`、`src/api/streaming/` — 实例中继与 SSE（v4 运行沿用同一 `/events` 通道，负载由节点按运行版本给出）
- `src/api/brain_runs/` — schema 7 计划与运行；`plan_capabilities.rs` 注册保存计划版本能力，`v4/` 实现目录负责准入、派发、读取和事件确认。
- [brain_runs/maintenance.rs](../../crates/control/src/api/brain_runs/maintenance.rs) — 删除计划或能力前分页检查全部保存版本与未结束 Brain 运行的冻结引用；保存、删除持有目录独占锁，运行准入持有共享锁至冻结请求提交。计划版本及元数据在 Store 事务内删除；[brain.rs](../../crates/control/src/api/brain.rs) 清理能力及目标绑定，重试可收尾残留元数据。
- [api/brain_runs/tui.rs](../../crates/control/src/api/brain_runs/tui.rs) — `GET /api/tui/agent-capabilities` 复用能力库目录，仅投影可用 Agent/Operator 的 ID、种类、目标和摘要；editor 与 viewer 可读，不返回定义与 Harness 私有设置。
- `src/transport/layered_tests.rs` — 分层 wake 的 generation 栅栏（只确认本次激活准入的那一轮）
- `src/scheduler.rs`、`src/scheduler/telemetry.rs`、`src/api/schedules/`、`src/api/scheduler_metrics.rs`、`src/seed_schedules.rs` — cron 调度、进程内计数、定义 CRUD、调度总览与遗留导入；[scheduler/timing.rs](../../crates/control/src/scheduler/timing.rs) 以定义生效时间和最近触发记录计算补跑、跳过与重试，纯函数不执行 I/O。定义 CRUD 的相同内容写入保留原生效时间。
- [scheduler.rs](../../crates/control/src/scheduler.rs) 按定义 ID 保留一个进行中的触发任务，跨扫描去重；各定义独立等待准入，慢请求不阻塞其他日程。重启后的去重仍依赖持久化派发记录与确定性执行 ID；回归见 [schedule_api/isolation.rs](../../crates/control/tests/e2e/schedule_api/isolation.rs)。
- 调度失败若已有持久化 `rejected` 派发回执，保留错误台账并按下一到期时间生成新执行；未确定的失败在一小时内沿原 ID 重试。Brain 保留独立入口的重试规则；回归见 [schedule_api/retry.rs](../../crates/control/tests/e2e/schedule_api/retry.rs)。
- `src/bootstrap.rs`、`src/routes.rs` — 管理凭据与独立指标凭据分开装配；指标凭据只允许读取 `GET /metrics`，相同凭据拒绝启动
- [resource_scope.rs](../../crates/control/src/resource_scope.rs)、[release/resources.rs](../../crates/control/src/release/resources.rs) — 固定资源服务配置并装配共享二进制管理接口及四个只读 NFS 导出；版本 Server 转发管理请求，保留查询参数、原始文件字节与响应元数据。
- [ontology.rs](../../crates/control/src/ontology.rs)、[routes.rs](../../crates/control/src/routes.rs) — 挂载 `/api/ontology` 和正文 NFS 管理接口；正文根与 Agent、二进制、源工作区互不包含，不能导出 Server 数据库。域实现见 [ontology](../ontology/index.md)。
- `tests/e2e/` — 集成测试（含 `layered_api` 家族：锁定读面、命令门禁、嵌套准入）

## 接缝
- [brain_runs/v4/gateway.rs](../../crates/control/src/api/brain_runs/v4/gateway.rs) 将根请求中显式给出的物理 `request.node_id` 传给普通托管子执行；未指定时仍交由统一调度选择节点，不使用根执行选中的 `index.node_id` 或计划逻辑节点 ID 代替。两类合同及重复投递保全见 [placement_tests.rs](../../crates/control/src/transport/layered_tests/placement_tests.rs)。嵌套 Brain 的父节点合同单独保留。
- Brain 根运行另要求 `brain_context_budget_v1`：`api/executions/capabilities.rs` 用实际请求探测模型容量，`submit.rs` 保留容量拒绝的 413；创建和唤醒均检查帧大小。超限唤醒明确阻塞，避免不断重试无法传输的上下文。先升级节点再启用 Server，预算实现见 [brain](../brain/index.md)。
- 新 DAG、根 Brain 与托管子执行还要求 `execution_io_v1`。[v4/read.rs](../../crates/control/src/api/brain_runs/v4/read.rs) 从子执行所属节点解析 output/artifacts 的 JSON Pointer；[v4/runtime.rs](../../crates/control/src/api/brain_runs/v4/runtime.rs) 把文件引用放入决策 context 并复核容量。索引与上下文按根 run 归属，原文件始终由其所属节点提供，恢复不改派原执行。
- Brain 里程碑计划：根运行及托管子执行的节点须同时广告 `brain_scheduler_v7`、`brain_contracts_v1`；保存版本和直接运行准入均规范化内部层间路径，旧版本显式路径保持原样。普通能力走统一执行提交，子计划走相同 Brain 准入并核验父 operation，且沿用父运行的节点。节点持有运行与操作投影；视图按轮次和激活提供执行索引，详情由执行 ID 查询。Control 为每次激活解析冻结能力与完整有界结果；受理拒绝从根事件读取具体原因，不等待不存在的子执行。Worker 执行模型决策；子计划固定版本并验证父 operation、深度和终态。目录解析失败只标记对应能力不可用，计划引用它时返回原因，不阻断其他计划。字段约定与回执入口见 [brain](../brain/index.md)。
- [api/executions/submit.rs](../../crates/control/src/api/executions/submit.rs) 接收用户注册的工作流定义，受理时原样固定定义与输入；找不到目标时拒绝提交，不按能力名称改写工作流。
- [api/brain_runs/attachments](../../crates/control/src/api/brain_runs/attachments/mod.rs) 提供通用图片附件上传与读取，核验名称、实际图片内容和 MIME；附件引用包含摘要，不依赖特定任务表单。
- `POST /api/brain/runs/:id/inputs` 将人工文本提交给根运行的事件日志。`v4/runtime.rs` 组装新上下文，`v4/delivery.rs` 对当前运行中的 Agent/Operator/Team 投递大脑确认的引导；后到的人工输入会使尚未投递的旧引导失效，投递与输入共用运行锁。
- `src/api/admission.rs` — `/ready` 在开放模式读取准入与节点就绪快照；冻结模式读取完整 drain 状态和活动执行数。
- [api/executions/capabilities.rs](../../crates/control/src/api/executions/capabilities.rs) — 静态与动态 DAG 均要求 `dag_container_v1`，动态还要求 `dag_dynamic_v1`；通用兼容响应不能代替原生容器能力。定义由用户维护，启动不自动注册专用应用 DAG。

## 相关
- [agents/node](../node/index.md)、[agents/worker](../worker/index.md)
- [dag-binary](../dag-binary/index.md)、[DAG 执行约定](../../rules/04-dag-execution-contract.md)
- [agents/brain](../brain/index.md)、[运行协议](../../docs/brain-orchestration.md)

## 私有任务文件

`src/api/executions/private_context.rs` 接收 DAG 专用私有文件合同，公开 request 不含文件；定义先校验后预留，私有内容参与幂等指纹。`/api/nodes/{id}/execution-capabilities` 显式探测节点执行文件摘要；Hub 仅对该探测使用 60 秒窗口，普通探测保留 15 秒。

## 派发与报告

- [outbox.rs](../../crates/control/src/release/outbox.rs)、[retry.rs](../../crates/control/src/release/outbox/retry.rs)：按执行 ID 退避恢复派发，节点代次变化立即重试，保留冻结的私有上下文。
- [outbox/resources.rs](../../crates/control/src/release/outbox/resources.rs)：资源任务受理后先持久化等待，按创建时间及 ID 扫描，获得外部授权后才创建节点执行；不可运行项不阻挡其他项。普通任务保持持久化前预留节点容量的路径，不依赖资源提供方。
- [socket.rs](../../crates/control/src/transport/socket.rs) 顺序处理连接的应用帧；[socket_io.rs](../../crates/control/src/transport/socket_io.rs) 独立处理心跳和写出，用 128 项有界队列及一项待入队帧施加背压。队列满时仍可发送 RPC 与交接确认。输入活性使用独立的 20 秒截止时间，仅收到有效帧才续期；发送数据与队列满都不能延后它。截止后沿原断线路径解除连接，节点可用同一代次重连，原执行归属与持久结果保持。通道数据库调用逐项等待阻塞线程的结果，避免同步 SQLite 占住 Tokio 线程；心跳只更新在线时间。Host 库存提交并通过代次栅栏后才标记就绪，交接确认通过独立的一项队列等待实际写出。断线取消所属连接的处理 future，迟到的数据库结果不能确认替代连接。
- [v4/view.rs](../../crates/control/src/api/brain_runs/v4/view.rs)：层调度理由只取本层 `layer_started` 事件，不被执行终态覆盖。

索引报告在 Begin 固定其负载快照的 sequence；报告中间的 RPC 负载快照可以更新连接视图，但不会抬高较早报告的持久水位。被忽略的旧 Begin 也不能替换当前报告水位。初始 pending 捕获和完整报告落库的慢调用记录节点、报告 ID、条数和耗时，不记录帧 payload 或凭据。

协商后的报告按 Begin、批次、Brain 帧和 End 确认，End 仅在原子索引提交及连接代次栅栏通过后确认。一条连接最多保留一个报告提交任务；数据库等待时继续消费该连接的回复与负载快照。该任务属于连接，断线会取消等待，迟到的存储完成不能标记替代连接就绪或发送交接确认。旧 Node 不声明 `acknowledge` 时沿原顺序处理，不接收额外确认。确认复用独立的一项写出队列，不占 RPC 队列或改变配额。

TODO 文件由 `project_files` 提供列表、流式上传、下载和删除；任务删除须先核验没有文件。只读 NFS 生命周期由专用 `opencoder-todo-resources.service` 管理，Server 仅转发管理接口，HTTP 文件操作使用同一绑定目录。见 [任务文件](../../docs/project-files.md)。

- [brain_runs/runs.rs](../../crates/control/src/api/brain_runs/runs.rs) 的运行历史支持 `limit`（默认 20）和原有游标，通过执行名称批量查询增加可选 `name`。通用执行列表和单条索引使用同一冻结计划名称，不访问当前计划库或 Worker；五字段执行索引协议不变。

能力目录区分内置、登记、自动生成和计划来源。`GET /api/brain/library` 返回完整 `capabilities` 及新选择使用的 `selection_ids`；仅执行类型、目标、解析定义和必填输入输出集合相同的自动项让位于登记项。TUI 使用同一规则，准入仍读取完整目录。见 [catalog](../../crates/control/src/api/brain_runs/catalog.rs)。

## 目录写入与清理

[catalog_guard](../../crates/control/src/catalog_guard/mod.rs) 在资源代理之前检查 DAG、二进制和能力目标写入。发布与删除持有跨进程目录独占锁，引用写入及首次执行冻结持有共享锁；共享锁最多等待 30 秒，超时返回可重试的 503。删除检查保存计划、能力、日程、TODO、活动执行、待派发回执、配置与 Agent 文件。Agent 文件不按扩展名跳过；无法读取、含 NUL、不是 UTF-8 或超过 4 MiB 的文件及失效资源根都会阻止删除。健康接口声明 `catalog_guard: 2`；新维护工具拒绝旧版保护。GET 详情提供 ETag，条件删除在锁内复核。删除标记阻止晚到请求引用已删目标，原名重新创建后可再次使用。独立资源服务仍负责文件与 NFS 生命周期。

[能力目录投影](../../crates/control/src/api/brain_runs/catalog/dedup.rs) 折叠同目标、同定义与同字段约定的条目；跨名 Agent 还核验实际资源的全部文件、相对路径与执行配置。资源读取和摘要计算在阻塞线程中执行，并显式保留请求的 Agent 资源根，避免占用处理准入与健康检查的异步线程。其他说明及 ID 保留在 `equivalent_sources`；接口仍返回完整目录，`selection_ids` 仅控制新建选项，旧计划编辑和执行解析保留原 ID。
