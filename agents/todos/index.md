Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# todos 模块

执行预编译 WorkflowSpec 的持久化 TODO 工作流运行时；每 TODO 独立 Primary Session。

## 索引
- `src/types.rs`、`src/domain.rs` — WorkflowSpec 与校验
- `src/parent.rs` — 父 workflow 决策循环
- `src/execution.rs`、`src/batch.rs` — TODO 派发与闭环
- `src/transitions.rs`、`src/persistence.rs` — 状态机守卫与 generation CAS
- `src/directory/` — JSON/Markdown 文件集 ↔ WorkflowSpec
- store 表 `todo_workflows`/`todo_items`/`todo_events`

## 相关
- [features/todos](../../features/todos/index.md) — 操作面
