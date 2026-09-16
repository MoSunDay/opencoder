Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# worker 模块

节点执行面：接受/恢复执行、资源快照、workload 适配。

## 索引
- `crates/worker/src/service.rs` — 根执行与会话清单
- `crates/worker/src/workloads/` — agent/team/dag/todos/project 适配器
- `crates/worker/src/state.rs` — runtime.db 与节点 ID
- `crates/worker/src/layout.rs`、`src/journal/` — 执行布局与原子落盘
- `crates/worker/src/runtime/capacity.rs`、`src/resources.rs` — Runtime 归属与资源固定

## 相关
- [agents/node](../node/index.md)、[agents/dag-runtime](../dag-runtime/index.md)
