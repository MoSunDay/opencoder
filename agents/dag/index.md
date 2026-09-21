Commit: fc047704e4c583cb9e0c11293b3815916ce1659b

# dag 模块

DAG 纯域 + 线协议；DTO LOCKED，无 IO，线协议变更需跨端同步。

## 索引
- `src/spec.rs`、`src/domain.rs` — 执行步骤与 Dynamic 模板节点定义/校验
- `src/policies.rs` — 整跑 `max_concurrency` 的纯策略：缺省 4，合法范围 1–30；API 与节点调度使用同一 spec 字段。
- `src/dynamic.rs` — 动态节点展开与批次校验（纯函数）
- `src/artifacts.rs` — 逻辑节点与实例路径约束
- `src/protocol.rs` — 生命周期事件

## 相关
- [dag-runtime](../dag-runtime/index.md) — 执行与原子持久化
- [动态步骤](../../docs/dag-dynamic.md) — 实例 API 与恢复契约
