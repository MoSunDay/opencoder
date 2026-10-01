Commit: 7687b5f581254ee6d826d8644789e7d498e761ba

# dag 模块

DAG 纯域 + 线协议；DTO LOCKED，无 IO，线协议变更需跨端同步。

## 索引
- `src/spec.rs`、`src/domain.rs` — Binary、Agent 与 Dynamic 模板定义；二进制资源引用和参数数组严格校验，拒绝未知字段与未知执行类型。
- `src/policies.rs` — 整跑 `max_concurrency` 的纯策略：缺省 4，合法范围 1–30；API 与节点调度使用同一 spec 字段。
- `src/dynamic.rs` — 动态节点展开与批次校验（纯函数）
- `src/artifacts.rs` — 逻辑节点、实例路径与保留目录名称约束；动态实例使用 `<step>/instances/<index>`。
- `src/protocol.rs` — 生命周期事件

## 相关
- [dag-runtime](../dag-runtime/index.md) — 执行与原子持久化
- [执行约定](../../rules/04-dag-execution-contract.md)、[DAG 能力](../../features/dag/index.md)
- [动态步骤](../../docs/dag-dynamic.md) — 实例 API 与恢复契约
