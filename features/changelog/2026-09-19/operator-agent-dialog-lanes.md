Commit: 6c6ad7442dc043f9040f20f7d0aa29287ebda887

# Operator 与 Agent 会话记录分 lane

## 变更

- `GET /api/sessions` 与 `/api/nodes/:id/dialogs` 支持 `kind=operator|agent`，缺省仍为 Operator；列表不再把两类执行混在一起。
- 对话行返回 `kind`、`node_id` 和 `execution_ref`。Agent 执行继续落在可执行 Agent 的 Operator-capable 节点，由该节点保留记录并路由明细。
- 批量删除只作用于当前 lane，节点只收到当前类型的候选 id；节点报告跳过的记录保留控制面索引。
- Web 切换 Operator/Agent 时重新加载对应 lane，并忽略之前模式的迟到列表与创建响应；Agent 模式保留具体 Agent 选择和统一 Say/transcript 输出。

## 验证

- control e2e：sessions 18 passed，dialog lane delete passed。
- SPA Vitest：114 files / 844 tests passed。
- `npm run build`：完成，已更新 `crates/web/spa/dist/static/app.js`。
