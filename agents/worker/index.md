Commit: 7e71cbcfd669dd2cbaa5c94ab01945fd139557f0

# worker 模块

节点执行面：接受/恢复执行、资源快照、workload 适配。

## 索引
- `crates/worker/src/service.rs` — 根执行与会话清单
- `crates/worker/src/workloads/` — agent/team/dag/todos/project 适配器
- `crates/worker/src/workloads/agent_how.rs`、`agent_runc.rs`（+ `agent_runc/`）— how 契约与 `run_mode: agent` runc 运行时（准入 fail-closed）
- `crates/worker/src/operations/` — 准入/launch/维护命令/查询（含 `query/instances/`、`artifacts.rs`）
- `crates/worker/src/state.rs`、`src/layout.rs`、`src/journal/` — runtime.db、执行布局与原子落盘
- `crates/worker/src/runtime/`、`src/resources.rs` — Runtime 归属与资源快照
- `crates/worker/src/brain/` — 图激活、路由、回执、输出适配与唤醒（`workdir.rs` 工作空间接缝）
- `tests/` — 集成测试

## 接缝
- DAG 的 how 追加由 dag-runtime 写入本地副本；普通 Agent 会话资源追加由 `agent_how.rs` 管理。
- Brain v3：worker 根节点持调度 projection/generation；control 只创建子执行并处理中继回执。

## 相关
- [agents/node](../node/index.md)、[agents/dag-runtime](../dag-runtime/index.md)
- [agents/brain](../brain/index.md)
- [运行协议](../../docs/brain-orchestration.md)、[动态步骤接口](../../docs/dag-dynamic.md)
