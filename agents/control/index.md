Commit: 7687b5f581254ee6d826d8644789e7d498e761ba

# control 模块

平台控制面：节点接入、全局定义、执行索引与调度。

## 索引
- `src/bootstrap.rs` — control.db + definitions.db 装配
- `src/transport/hub.rs` — Node WS Hub（协议校验、RPC）
- `src/api/session.rs` — 会话执行面（`?kind=operator|agent` 泳道）
- `src/api/executions/`、`src/role_gate.rs` — 派发去重、选点冻结、回执与角色门禁；执行索引五字段协议
- `src/api/settings/` — Harness 配置与节点调度配置（Maintenance RPC 转发）
- `src/api/catalog.rs`、`src/api/compat/`、`src/api/project.rs` — 节点/定义目录、兼容路由与 Project 中继；项目、专项、TODO、Tag 处理器复用 Web 实现并经 `src/routes.rs` 装配
- `src/api/project_links.rs`、`src/scheduler/project_assignments.rs` — TODO 关联 Agent、Operator、Team、DAG、TODO 工作流或 Brain 执行；定期从所属节点读取产物，回写独立结论及同步状态
- `src/api/compat/dag_instances.rs`、`src/api/stream.rs`、`src/api/streaming/` — 实例中继与 SSE（v4 运行沿用同一 `/events` 通道，负载由节点按运行版本给出）
- `src/api/brain_runs/` — schema 7 计划与运行；`plan_capabilities.rs` 注册保存计划版本能力，`v4/` 实现目录负责准入、派发、读取和事件确认。
- [api/brain_runs/tui.rs](../../crates/control/src/api/brain_runs/tui.rs) — `GET /api/tui/agent-capabilities` 复用能力库目录，仅投影可用 Agent/Operator 的 ID、种类、目标和摘要；User 可读，不返回定义与 Harness 私有设置。
- `src/transport/layered_tests.rs` — 分层 wake 的 generation 栅栏（只确认本次激活准入的那一轮）
- `src/scheduler.rs`、`src/scheduler/telemetry.rs`、`src/api/schedules/`、`src/api/scheduler_metrics.rs`、`src/seed_schedules.rs` — cron 调度、进程内计数、定义 CRUD、调度总览与遗留导入
- `src/bootstrap.rs`、`src/routes.rs` — 管理凭据与独立指标凭据分开装配；指标凭据只允许读取 `GET /metrics`，相同凭据拒绝启动
- [resource_scope.rs](../../crates/control/src/resource_scope.rs)、[release/resources.rs](../../crates/control/src/release/resources.rs) — 固定资源服务配置并装配共享二进制管理接口及三个只读 NFS 导出；版本 Server 转发管理请求，保留查询参数、原始文件字节与响应元数据。
- `tests/e2e/` — 集成测试（含 `layered_api` 家族：锁定读面、命令门禁、嵌套准入）

## 接缝
- Brain 里程碑计划：节点须广告 `brain_scheduler_v7`；保存版本和直接运行准入均规范化内部层间路径，旧版本显式路径保持原样。普通能力走统一执行提交，子计划走相同 Brain 准入并核验父 operation，且沿用父运行的节点。节点持有运行与操作投影；视图按轮次和激活提供执行索引，详情由执行 ID 查询。Control 为每次激活解析能力及有界上游摘要，Worker 执行模型决策；子计划固定版本并验证父 operation、深度和终态。目录解析失败只标记对应能力不可用，计划引用它时返回原因，不阻断其他计划。PC 安装入口保留旧 schema 版本，追加并幂等返回最新 schema 7 版本。
- `POST /api/brain/runs/:id/inputs` 将人工文本提交给根运行的事件日志。`v4/runtime.rs` 组装新上下文，`v4/delivery.rs` 对当前运行中的 Agent/Operator/Team 投递大脑确认的引导；后到的人工输入会使尚未投递的旧引导失效，投递与输入共用运行锁。
- `src/api/admission.rs` — `/ready` 在开放模式读取准入与节点就绪快照；冻结模式读取完整 drain 状态和活动执行数。
- [api/executions/capabilities.rs](../../crates/control/src/api/executions/capabilities.rs) — 静态与动态 DAG 均要求 `dag_container_v1`，动态还要求 `dag_dynamic_v1`；通用兼容响应不能代替原生容器能力。定义由用户维护，启动不自动注册评审 DAG。

## 相关
- [agents/node](../node/index.md)、[agents/worker](../worker/index.md)
- [dag-binary](../dag-binary/index.md)、[DAG 执行约定](../../rules/04-dag-execution-contract.md)
- [agents/brain](../brain/index.md)、[运行协议](../../docs/brain-orchestration.md)

## 私有任务文件

`src/api/executions/private_context.rs` 接收 DAG 专用私有文件合同，公开 request 不含文件；定义先校验后预留，私有内容参与幂等指纹。`/api/nodes/{id}/execution-capabilities` 显式探测节点执行文件摘要；Hub 仅对该探测使用 60 秒窗口，普通探测保留 15 秒。

## 派发与报告

- [outbox.rs](../../crates/control/src/release/outbox.rs)、[retry.rs](../../crates/control/src/release/outbox/retry.rs)：按执行 ID 退避恢复派发，节点代次变化立即重试，保留冻结的私有上下文。
- [socket.rs](../../crates/control/src/transport/socket.rs)：Host 库存持久化并通过代次栅栏后直接发送交接确认，避免 socket 循环等待自己消费的满队列。
- [v4/view.rs](../../crates/control/src/api/brain_runs/v4/view.rs)：层调度理由只取本层 `layer_started` 事件，不被执行终态覆盖。
