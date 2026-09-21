Commit: 3b4775905c950f64433b5c9f4439f4396674b3c6

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
- `src/api/brain_runs/` — 仅 v4 计划与运行；`plan_capabilities.rs` 注册保存计划版本能力，`v4/` 负责准入、派发、读取和事件确认。
- `src/transport/layered_tests.rs` — v4 wake 的 generation 栅栏（只确认本次激活准入的那一轮）
- `src/scheduler.rs`、`src/api/schedules/`、`src/seed_schedules.rs` — cron 调度、定义 CRUD 与遗留导入
- `src/seed_dags.rs` — review 门禁 def seed
- `tests/e2e/` — 集成测试（含 v4 `layered_api` 家族：锁定读面、命令门禁、嵌套准入）

## 接缝
- Brain 分层能力计划：节点须广告 `brain_scheduler_v4`；普通能力走统一执行提交，子计划走相同 Brain 准入并核验父 operation。层级纯函数重算，节点持有运行与操作投影；视图仅提供索引，详情由能力查询。Control 为每次激活解析能力及有界上游摘要，Worker 执行模型决策；子计划固定版本并验证父 operation、深度和终态。

## 相关
- [agents/node](../node/index.md)、[agents/worker](../worker/index.md)
- [agents/brain](../brain/index.md)、[运行协议](../../docs/brain-orchestration.md)
