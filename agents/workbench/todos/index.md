Commit: cea9499682d959503567277918b3f8da3d8115ba

# todos 模块

执行预编译 `WorkflowSpec` 的持久化 TODO 工作流运行时；每 TODO 独立
Primary Session。

## 索引
- `crates/todos/src/types.rs`、`domain.rs` — `WorkflowSpec` 与校验
- `crates/todos/src/parent.rs` — 父 workflow 决策循环
- `crates/todos/src/execution.rs`、`batch.rs` — TODO 派发与闭环
- `crates/todos/src/transitions.rs`、`persistence.rs` — 状态机守卫、
  generation CAS、store 表 `todo_*`
- `crates/todos/src/directory/` — 文件集 ↔ WorkflowSpec 无损编解码
- `scripts/e2e/todos_*.py` — e2e 场景回归

## 相关
- [features/todos](../../../features/platform/todos/index.md) — 操作面
