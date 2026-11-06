Commit: a8a742ba7c4b952173e1403d3e11da809d3f80cf

# TODO 工作台

JSON/Markdown 目录编辑、运行、Review 与指定节点重跑；本卡只做指路。

## 索引

- [todos 域卡](../../../agents/workbench/todos/index.md) — `WorkflowSpec` 运行时、状态机
  与目录编解码
- [crates/todos/src/directory/](../../../crates/todos/src/directory) — 文件集 ↔
  WorkflowSpec 无损编解码
- [crates/todos/src/runner.rs](../../../crates/todos/src/runner.rs)、
  [review/](../../../crates/todos/src/review) — 运行与 Review
- [workloads/todos.rs](../../../crates/worker/src/workloads/todos.rs)
  — 节点执行
- [crates/web/src/api_todo_*.rs](../../../crates/web/src/api_todo_runs.rs) —
  工作台、模板与运行 API
- [crates/local/src/todos_cmd.rs](../../../crates/local/src/todos_cmd.rs) — 本地
  TODO 命令
- [操作文档](../../../docs/todo-workbench.md)

## 相关

- [agents/todos](../../../agents/workbench/todos/index.md) — 运行时
- [agents/worker](../../../agents/execution/worker/index.md) — 节点执行
- UI 验收：[目录与运行](../../../scripts/acceptance/todo_workbench/main.js)、
  [初始化与停止状态](../../../scripts/acceptance/todo_initialization_ui.js)
- [项目工作台](../project/index.md)
