Commit: d26f8cb5a16a52072daad02c77ae63161973526b

# control 模块

平台控制面：节点接入、全局定义、执行索引与调度。

## 索引
- `src/bootstrap.rs` — control.db + definitions.db 装配
- `src/transport/hub.rs` — Node WS Hub（协议校验、RPC）
- `src/api/session.rs` — 会话执行面（`?kind=operator|agent` 泳道）
- `src/api/executions/`、`src/role_gate.rs` — 派发去重、选点冻结、回执与角色门禁；执行索引五字段协议
- `src/api/settings/` — Harness 配置与节点调度配置（Maintenance RPC 转发）
- `src/api/catalog.rs`、`src/api/compat/`、`src/api/project.rs` — 节点/定义目录、兼容路由与 Project 中继
- `src/api/compat/dag_instances.rs`、`src/api/stream.rs`、`src/api/streaming/` — 实例中继与 SSE（v4 运行沿用同一 `/events` 通道，负载由节点按运行版本给出）
- `src/api/brain_runs/` — schema 6 计划与运行；`plan_capabilities.rs` 注册保存计划版本能力，`v4/` 实现目录负责准入、派发、读取和事件确认。
- `src/transport/layered_tests.rs` — 分层 wake 的 generation 栅栏（只确认本次激活准入的那一轮）
- `src/scheduler.rs`、`src/api/schedules/`、`src/seed_schedules.rs` — cron 调度、定义 CRUD 与遗留导入
- `src/seed_dags.rs` — 仅初始化 `review-harness-quick`；按名已存在则跳过，保留操作者定义
- `tests/e2e/` — 集成测试（含 `layered_api` 家族：锁定读面、命令门禁、嵌套准入）

## 接缝
- Brain 里程碑计划：节点须广告 `brain_scheduler_v4`（现存协议能力名）；普通能力走统一执行提交，子计划走相同 Brain 准入并核验父 operation。节点持有运行与操作投影；视图按轮次和激活提供执行索引，详情由执行 ID 查询。Control 为每次激活解析能力及有界上游摘要，Worker 执行模型决策；子计划固定版本并验证父 operation、深度和终态。
- `src/api/admission.rs` — `/ready` 在开放模式读取准入与节点就绪快照；冻结模式读取完整 drain 状态和活动执行数。

## 相关
- [agents/node](../node/index.md)、[agents/worker](../worker/index.md)
- [agents/brain](../brain/index.md)、[运行协议](../../docs/brain-orchestration.md)

## 私有任务文件

`src/api/executions/private_context.rs` 接收 DAG 专用私有文件合同，公开 request 不含文件；定义先校验后预留，私有内容参与幂等指纹。`/api/nodes/{id}/execution-capabilities` 显式探测节点执行文件摘要；Hub 仅对该探测使用 60 秒窗口，普通探测保留 15 秒。

## 派发与报告

- [outbox.rs](../../crates/control/src/release/outbox.rs)、[retry.rs](../../crates/control/src/release/outbox/retry.rs)：按执行 ID 退避恢复派发，节点代次变化立即重试，保留冻结的私有上下文。
- [socket.rs](../../crates/control/src/transport/socket.rs)：Host 库存持久化并通过代次栅栏后直接发送交接确认，避免 socket 循环等待自己消费的满队列。
- [v4/view.rs](../../crates/control/src/api/brain_runs/v4/view.rs)：层调度理由只取本层 `layer_started` 事件，不被执行终态覆盖。
