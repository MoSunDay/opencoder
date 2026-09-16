Commit: 30108c8be3b60a11d2a6b41b0b9b482529678209

# todos 模块

执行预编译 WorkflowSpec 的持久化 TODO 工作流运行时；每 TODO 独立 Primary Session。

## 索引
- `src/types.rs`、`src/domain.rs` — WorkflowSpec 与校验
- `src/parent.rs` — 父 workflow 决策循环
- `src/execution.rs`、`src/batch.rs` — TODO 派发与闭环
- `src/transitions.rs`、`src/persistence.rs` — 状态机守卫与 generation CAS
- `src/directory/` — JSON/Markdown 文件集 ↔ WorkflowSpec
- store 表 `todo_workflows`/`todo_items`/`todo_events`

## e2e 套件（scripts/e2e/）
- `todos_contract_scenarios.py`（E21，无 key）— validate 诊断归因、env.json 绑定、观测面与挂起记录契约
- `todos_runtime_scenarios.py`（E22，需 key）— 输出文档/事件目录/游标、确定性失败闭环、本地 Ctrl-C 130 + `--debug` 投影、目录格式与 env 透传
- `todos_scenarios.py`（E19b/E19c，需 key）— 成功主流程 / revise 重试 / 中断恢复
- `todos_web_scenarios.py`（E23）— serve 端模板/环境生命周期（HARD）与无节点 run 拒绝契约（compat 语义）

## 相关
- [features/todos](../../features/todos/index.md) — 操作面
