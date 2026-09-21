Commit: 7e71cbcfd669dd2cbaa5c94ab01945fd139557f0

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
- `src/api/brain_runs/` — Brain v2/v3 计划、轮次调度与事件页（`schema_version: 3`）；`v4/` 为分层能力画布（`schema_version: 4`）的准入/读取/投递与锁定的 `/layered[/rounds/:round]` 读面
- `src/transport/layered_tests.rs` — v4 wake 的 generation 栅栏（只确认本次激活准入的那一轮）
- `src/scheduler.rs`、`src/api/schedules/`、`src/seed_schedules.rs` — cron 调度、定义 CRUD 与遗留导入
- `src/seed_dags.rs` — review 门禁 def seed
- `tests/e2e/` — 集成测试（含 v4 `layered_api` 家族：锁定读面、命令门禁、嵌套准入）

## 接缝
- Brain v3：control 只归一化能力目录、创建子执行并处理中继回执；调度 projection/generation 留在 worker 根节点。
- Brain v4（分层能力画布）：准入按 `schema_version` 分支（3→v3、4→v4、其余 409 `migration required`），节点须广告 `brain_scheduler_v4`；控制面只归一化能力目录、创建带 `input.brain_layered` 的普通子执行、转发 `layered_wake|layered_dispatch|layered_cancel|layered_terminal` 与对应 ack。层划分从不落库，读面用 `opencoder_brain::layered` 重算；运行投影与 generation 留在 worker 根节点，v3 与 v4 的读路径互不跨服（非本版本运行 404）。

## 相关
- [agents/node](../node/index.md)、[agents/worker](../worker/index.md)
- [agents/brain](../brain/index.md)、[运行协议](../../docs/brain-orchestration.md)
