Commit: 30108c8be3b60a11d2a6b41b0b9b482529678209

# todos 模块

执行预编译 WorkflowSpec 的持久化 TODO 工作流运行时；每 TODO 独立 Primary Session。

## 索引
- `src/types.rs`、`src/domain.rs` — WorkflowSpec 与校验
- `src/parent.rs` — 父 workflow 决策循环
- `src/execution.rs`、`src/batch.rs` — TODO 派发与闭环
- `src/transitions.rs`、`src/persistence.rs` — 状态机守卫、generation CAS、store 表 `todo_*`
- `src/directory/` — 文件集 ↔ WorkflowSpec
- `scripts/e2e/todos_*.py` — e2e 场景（E19b/c、E21–E23）

## 相关
- [features/todos](../../features/todos/index.md) — 操作面
