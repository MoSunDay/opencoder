Commit: 35377f739c15039b725df8c135a9a14cec8bf815

# Agent 会话选择与批量清理

## 变更

- Agent 模式移除「知识追加」入口，页面改为明确选择具体的 primary Agent，并显示其描述；首条需求仍沿用会话统一 transcript/Say 渲染，便于定位实际执行能力。
- Agent 会话创建时，首条 prompt 在节点执行侧作为该 Agent 的初始 how 内容注入并在成功后持久化；Operator 会话保持原有请求形状。
- `/api/nodes/:id/dialogs` 批量清理同时收集 Operator 与 Agent 索引，终态会话统一转发给节点清理并按执行 kind 删除控制面索引，运行中会话继续保留。

## 验证

- `cargo test -p opencoder-control --test e2e -- --nocapture`：188 passed。
- `cargo test -p opencoder-worker --test maintenance_dialogs_clear -- --nocapture`：2 passed。
- SPA 全量 Vitest：114 files / 841 passed。
- `npm run build`：完成，已更新 `crates/web/spa/dist/static/app.js`。
