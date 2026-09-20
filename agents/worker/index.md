Commit: efe243b640c6e550770a3373e48c80773297bdfb

# worker 模块

节点执行面：接受/恢复执行、资源快照、workload 适配。

## 索引
- `crates/worker/src/service.rs` — 根执行与会话清单
- `crates/worker/src/workloads/` — agent/team/dag/todos/project 适配器
- `crates/worker/src/workloads/agent_how.rs`、`agent_runc.rs`（+ `agent_runc/`）— how 契约与 `run_mode: agent` runc 运行时（准入 fail-closed）
- `crates/worker/src/operations/` — 准入/launch/维护命令/查询（含 `query/instances/`、`artifacts.rs`、`operator_env.rs` Operator 隔离快照）
- `crates/worker/src/state.rs`、`src/layout.rs`、`src/journal/` — runtime.db、执行布局与原子落盘（layout 含 `<data>/operator/<id>/{home,workspace}` 预留）
- `crates/worker/src/runtime/`、`src/resources.rs` — Runtime 归属与资源快照
- `crates/worker/src/brain/` — 图激活、路由、回执、输出适配与唤醒（`workdir.rs` 工作空间接缝）
- `tests/` — 集成测试

## Operator 执行隔离

`operations/operator_env.rs` 为 `ExecutionKind::Operator` 提供按执行的 HOME/WORKSPACE 隔离：`create()` 准入通过后把冻结配置快照（明文含 provider api key）以 0600 写入 `<data_dir>/operator/<id>/home/.opencoder/config.json`，`resolve()` 双门（快照 + workspace 目录同时存在）失败即回退节点级默认。`env_pairs()` 产出 HOME 覆盖对，fresh 会话在 how_append 之后注入 envs 尾部、经 harness envs 持久化，resume 由 `resume.rs` 重建 env_passthrough。配置加载走 core `Config::load_with_home`（候选链重定向到执行 home），web drain 栈经 `AppState.config_home` 穿参（`handle/drain.rs` `DrainContext`），执行目录由 `brain/workdir.rs` `session_dirs()` 统一裁定（Operator→workspace，缺失回退 node workdir）。Maintenance/Agent/Brain 不受影响。

## 接缝
- `operations/dag_preflight.rs` 使用本次冻结配置校验静态步骤和动态模板。runc 模式要求节点 rootfs 和 runc 可用；Codex Agent 额外校验 guest CLI 与节点登录目录，不检查 host CLI，也不要求原生 provider 凭证。实际执行和私有挂载由 dag-runtime 负责。
- DAG 的 how 追加由 dag-runtime 写入本地副本；普通 Agent 会话资源追加由 `agent_how.rs` 管理。
- Brain v3：worker 根节点持调度 projection/generation；control 只创建子执行并处理中继回执。

## 相关
- [agents/node](../node/index.md)、[agents/dag-runtime](../dag-runtime/index.md)
- [agents/brain](../brain/index.md)
- [运行协议](../../docs/brain-orchestration.md)、[动态步骤接口](../../docs/dag-dynamic.md)
