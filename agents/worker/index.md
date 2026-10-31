Commit: a8a742ba7c4b952173e1403d3e11da809d3f80cf

# worker 模块

节点执行面：受理/恢复执行、资源快照与 workload 适配（lib.rs 模块声明）。

Windows 节点只接受 `ExecutionKind::Operator`，用户边界见
[Windows](../../features/windows/index.md)。

## 入口与模块
- [lib.rs](../../crates/worker/src/lib.rs) — Node-owned execution adapters；进程内会话 API
- [service.rs](../../crates/worker/src/service.rs) — 根执行与会话清单（`indexes()`）
- [operations/](../../crates/worker/src/operations/mod.rs) — 准入、创建、launch、维护命令与查询
- [workloads/](../../crates/worker/src/workloads/mod.rs) — agent/team/dag/todos/project 执行驱动
- [journal.rs](../../crates/worker/src/journal.rs) — 执行记录入口；子模块见 [journal](journal.md)
- [state.rs](../../crates/worker/src/state.rs)、[layout.rs](../../crates/worker/src/layout.rs) — 节点状态与执行目录布局
- [runtime/](../../crates/worker/src/runtime/mod.rs)、[resources.rs](../../crates/worker/src/resources.rs) — 容量与健康准入、NFS 资源快照
- [inventory/](../../crates/worker/src/inventory/mod.rs) — 由 journal、数据库与活动运行合成的增量清单
- [result_reader/](../../crates/worker/src/result_reader/mod.rs) — 不唤醒归属 Runtime 的持久化结果读取
- [brain/](../../crates/worker/src/brain/mod.rs) — Brain 运行投影与回执；`brain/v4/` 为节点侧实现
- [maintenance_tools.rs](../../crates/worker/src/maintenance_tools.rs)、[migration.rs](../../crates/worker/src/migration.rs) — 维护工具与记录迁移

## 关键接缝
- [operations/queue/](../../crates/worker/src/operations/queue/mod.rs) — 节点持有的持久待办；节拍纯函数 `dispatch_delay`
- [operations/operator_env.rs](../../crates/worker/src/operations/operator_env.rs) — Operator HOME/WORKSPACE 隔离
- [operator_env/linux_identity.rs](../../crates/worker/src/operations/operator_env/linux_identity.rs) — Operator 专用 Linux 系统账号
- [operations/operator_config.rs](../../crates/worker/src/operations/operator_config.rs) — 节点级 Operator 配置平面
- [operations/fork.rs](../../crates/worker/src/operations/fork.rs) — 派生会话按父执行类型生成 ID 与资源
- [operations/dag_preflight.rs](../../crates/worker/src/operations/dag_preflight.rs) — DAG 准入校验（镜像替换见 `image.rs`）
- [operations/query/session/](../../crates/worker/src/operations/query/session/mod.rs) — 持久受理到会话初始化之间的读取
- [workloads/agent_how.rs](../../crates/worker/src/workloads/agent_how.rs) — Agent 的 how 追加与结果投影（纯函数）
- [layout/dag.rs](../../crates/worker/src/layout/dag.rs)、[workloads/dag.rs](../../crates/worker/src/workloads/dag.rs) — DAG 目录与执行驱动
- [tests/](../../crates/worker/tests/) — 集成测试（Windows 准入见 `windows_operator.rs`）

## 相关
- [agents/node](../node/index.md)、[agents/dag-runtime](../dag-runtime/index.md)
- [DAG 执行约定](../../rules/04-dag-execution-contract.md)
- [agents/brain](../brain/index.md)
- [运行协议](../../docs/brain-orchestration.md)、[动态步骤接口](../../docs/dag-dynamic.md)
