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
- `src/api/compat/dag_instances.rs`、`src/api/stream.rs`、`src/api/streaming/` — 实例中继与 SSE
- `src/api/brain_runs/` — Brain v2/v3 计划、轮次调度与事件页（`schema_version: 3`）
- `src/scheduler.rs`、`src/api/schedules/`、`src/seed_schedules.rs` — cron 调度、定义 CRUD 与遗留导入
- `src/seed_dags.rs` — review 门禁 def seed
- `tests/` — 集成测试

## 接缝
- Brain v3：control 只归一化能力目录、创建子执行并处理中继回执；调度 projection/generation 留在 worker 根节点。

## 相关
- [agents/node](../node/index.md)、[agents/worker](../worker/index.md)
- [agents/brain](../brain/index.md)、[运行协议](../../docs/brain-orchestration.md)
