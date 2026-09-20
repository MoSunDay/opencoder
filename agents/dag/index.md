Commit: 7e71cbcfd669dd2cbaa5c94ab01945fd139557f0

# dag 模块

DAG 纯域 + 线协议。DTO LOCKED。

## 索引
- `src/spec.rs`、`src/domain.rs` — Agent/Wasm 执行步骤与 Dynamic 模板节点的定义和校验；模板不允许嵌套 Dynamic。
- `src/dynamic.rs` — JSON Pointer 来源、全批次类型/1,000 项上限校验、展开、进度计数和同组失败优先汇总，均为纯函数。
- `src/artifacts.rs` — 逻辑节点与 `(step, index)` 实例的路径约束。
- `src/protocol.rs` — 原有生命周期事件及 `instance_started`、`instance_done`、`step_progress`；快照和进度使用绝对计数。
- 执行和原子持久化由 [dag-runtime](../dag-runtime/index.md) 完成，接口规则见 [动态步骤](../../docs/dag-dynamic.md)。

## 边界
- 无 IO；线协议 DTO 变更需跨端同步。
