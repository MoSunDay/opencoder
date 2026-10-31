Commit: a8a742ba7c4b952173e1403d3e11da809d3f80cf

# dag 模块

DAG 纯域 + 线协议；DTO LOCKED，无 IO，线协议变更需跨端同步（lib.rs）。

## 索引
- [spec.rs](../../crates/dag/src/spec.rs) — 持久化工作流定义：步骤种类与依赖边
- [domain.rs](../../crates/dag/src/domain.rs) — 纯调度决策：校验、拓扑序、就绪集与上下文渲染，无 IO 无时钟
- [transitions.rs](../../crates/dag/src/transitions.rs) — 运行状态机，复用 `node_tasks` 的状态语义
- [policies.rs](../../crates/dag/src/policies.rs) — 调度策略纯函数：整跑 `max_concurrency` 的缺省与上限
- [dynamic.rs](../../crates/dag/src/dynamic.rs) — 动态节点输入校验、展开与进度聚合（纯函数）
- [artifacts.rs](../../crates/dag/src/artifacts.rs) — 节点本地产物目录契约（纯路径/slug/截断助手）
- [artifacts/declarations.rs](../../crates/dag/src/artifacts/declarations.rs) — `artifacts.json`/`report_archive` 声明校验，双端共用
- [layout.rs](../../crates/dag/src/layout.rs) — 按日期的运行父目录（`run_parent`）
- [protocol.rs](../../crates/dag/src/protocol.rs) — 节点侧 DAG 调度协议线 DTO，Server 链与 Agent 链共享
- [control/mod.rs](../../crates/dag/src/control/mod.rs) — 面向具体执行尝试的版本化操作命令与持久回执

## 相关
- [dag-runtime](../dag-runtime/index.md) — 执行与原子持久化
- [执行约定](../../rules/04-dag-execution-contract.md)、[DAG 能力](../../features/dag/index.md)
- [动态步骤](../../docs/dag-dynamic.md) — 实例 API 与恢复契约
