Commit: 40a688a77bfdbedc3f30f9f6b1e3a1ba67244d68

# worker 模块

节点执行面：接受/恢复执行、资源快照、workload 适配。

## 索引
- `crates/worker/src/service.rs` — 根执行与会话清单
- `crates/worker/src/workloads/` — agent/team/dag/todos/project 适配器
- `crates/worker/src/workloads/agent_how.rs`、`agent_runc.rs`（+ `agent_runc/`）— how 契约与 `run_mode: agent` runc 运行时（准入 fail-closed）
- `crates/worker/src/operations/` — 准入/launch/维护命令/查询（含 `query/instances/`、`artifacts.rs`、`operator_env.rs` Operator 隔离快照、`operator_config.rs` 节点级 Operator 配置平面）
- `crates/worker/src/state.rs`、`src/layout.rs`、`src/journal/` — runtime.db、执行布局与原子落盘（layout 含 `<data>/operator/<id>/{home,workspace}` 预留）
- `crates/worker/src/runtime/`、`src/resources.rs` — Runtime 归属与资源快照
- `crates/worker/src/brain/` — 图激活、路由、回执、输出适配与唤醒（`workdir.rs` 工作空间接缝）；`brain/v3/` 与 `brain/v4/` 为同一套接缝的两个版本模块
- `tests/` — 集成测试

## Operator 执行隔离

配置平面（P1）：`operations/operator_config.rs` 维护节点数据根下的 `<data>/operator-config/`（`config.json` + `mcp|cli|skills|ap|schedules` 五域文件 + `skills/` 技能包目录）。首个 Operator 执行准入时（`state.rs::configuration_for` → `operator_configuration`）从节点 live 视图 `bootstrap` 一次（0600 first-writer-wins），此后冻结：TUI/CLI 对共享 workdir 配置的保存不再进入 Operator 执行；技能冻结（P2）`freeze_skills` 把平面 `skills/` 包 + 内置 seed 写入执行 home（用户全局池永远不是来源）；`operations/launch.rs` 用 `core::skill::with_execution` 把 workload 包进执行技能根。

`operations/operator_env.rs` 为 `ExecutionKind::Operator` 提供按执行的 HOME/WORKSPACE 隔离：`create()` 准入通过后把冻结配置快照（明文含 provider api key）以 0600 写入 `<data_dir>/operator/<id>/home/.opencoder/config.json`，`resolve()` 双门（快照 + workspace 目录同时存在）失败即回退节点级默认。`env_pairs()` 产出 HOME 覆盖对，fresh 会话在 how_append 之后注入 envs 尾部、经 harness envs 持久化，resume 由 `resume.rs` 重建 env_passthrough。配置加载走 core `Config::load_with_home`（候选链重定向到执行 home），web drain 栈经 `AppState.config_home` 穿参（`handle/drain.rs` `DrainContext`），执行目录由 `brain/workdir.rs` `session_dirs()` 统一裁定（Operator→workspace，缺失回退 node workdir）。Maintenance/Agent/Brain 不受影响。

会话泳道（P4）：Operator 执行的 Primary Session 创建时打 `kind='operator'`（其他 kind 同理，见 store 索引），默认清单泳道排除 operator 行；`service.rs::indexes()` 按 `row.kind` 精确解析已打标行，存量 NULL 行保留 id 前缀/标题回退。

## 接缝
- `runtime/health.rs` 统一计算节点存储准入：可用磁盘块低于 10% 或可用 inode 低于 20% 拒绝新执行；容量读取失败、零容量仍拒绝准入。健康查询和新执行入口共用纯函数判断，已接收的工作可继续完成。
- `operations/dag_preflight.rs` 使用本次冻结配置校验静态步骤和动态模板。runc 模式要求节点 rootfs 和 runc 可用；Codex Agent 额外校验 guest CLI 与节点登录目录，不检查 host CLI，也不要求原生 provider 凭证。实际执行和私有挂载由 dag-runtime 负责。
- DAG 的 how 追加由 dag-runtime 写入本地副本；普通 Agent 会话资源追加由 `agent_how.rs` 管理。
- Brain v3：worker 根节点持调度 projection/generation；control 只创建子执行并处理中继回执。
- Brain：仅 `brain/v4/`，根节点持有运行、操作与事件投影；`layer` 是已派发层。每层并行、屏障后唤醒、重试耗尽失败并取消兄弟；空节点上下文作收口。既有 outbox 和 generation 保障恢复、重复回执幂等。
- 上限（`opencoder_brain::layered` 纯域校验）：`LAYERED_MAX_NODES=256`、`LAYERED_MAX_LAYER_WIDTH`=32/层、`LAYERED_MAX_DEPTH=3`、`retry.max_attempts` 1..=5（默认 2）。节点侧只做投影与栅栏校验（伪造/越权派发帧 409，迟到回执丢弃），层决策与准入裁决在 control。

## 相关
- [agents/node](../node/index.md)、[agents/dag-runtime](../dag-runtime/index.md)
- [agents/brain](../brain/index.md)
- [运行协议](../../docs/brain-orchestration.md)、[动态步骤接口](../../docs/dag-dynamic.md)
