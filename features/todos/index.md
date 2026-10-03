Commit: a7e6b8233d5a4f6d0727d656d5d3d45a985b4b63

# TODO 工作台

JSON/Markdown 目录编辑、运行、Review 与指定节点重跑。细节以代码与 [操作文档](../../docs/todo-workbench.md) 为准。

执行明细在工作流建立后展示工作台。准备期间显示初始化或停止状态；初始化失败展示原始错误，不读取不存在的工作台。详情准备完成后自动刷新，工作台读取失败仍报告实际错误。

## 相关
- [agents/todos](../../agents/todos/index.md) — 运行时
- [agents/worker](../../agents/worker/index.md) — 节点执行
- UI 验收：[目录与运行](../../scripts/acceptance/todo_workbench/main.js)、[初始化与停止状态](../../scripts/acceptance/todo_initialization_ui.js)
