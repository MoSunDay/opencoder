Commit: 225718daffe42e0ba369666b557ed062b114134a

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

## 回归补齐（2026-09-19 晚，766adbf4）

- 根包 `tests/operator_e2e/agent_session.rs` 对齐分 lane 契约：agent 会话仅出现在 `GET /api/sessions?kind=agent`，断言不漏入 operator lane（981a285f 漏改的测试期望，全量回归时确定性失败后修复）。
- 444e6b0e + 修复后全量回归：`cargo clippy --workspace --all-targets -- -D warnings` 通过；`cargo test --workspace` 426 套件 / 5376 passed / 0 failed / 7 ignored（runc 沙箱类按设计跳过）；`cargo build --workspace` 通过。

## 关闭遗漏（2026-09-19）

- `dialogs_clear` 在控制面、Host 休眠运行时和 Worker 三层统一携带 `kind`；缺省兼容旧 Operator 请求，跨 lane、活动或非终态记录拒绝/跳过，避免误删及索引复活。
- Agent 列表只显示用户创建的顶层会话。新 DAG/子 Agent 会话标记为 `agent_step`，历史 `dag/` 标题继续兼容过滤；这些执行记录仍通过父 Operator-capable 节点的 `execution_ref`/步骤明细访问。
- 批量删除增加 `dry_run=true` 只读预览，先按 lane 和可见性筛选，再由节点二次校验并删除终态记录；删除完成后仅清理对应 lane 的控制面索引。

验证：`cargo test -p opencoder-control --test e2e sessions_ -- --nocapture`（18 passed）、
`cargo test -p opencoder-control --test e2e dialogs_delete_ -- --nocapture`（3 passed）、
`cargo test -p opencoder-worker --test maintenance_dialogs_clear -- --nocapture`（3 passed）、
`cargo test -p opencoder-worker --test internal_session_index -- --nocapture`（2 passed）、
`cargo test -p opencoder-agent --bin opencoder-agent host_dialogs_clear -- --nocapture`（1 passed）、
`cargo test -p opencoder-dag-runtime --test run_loop -- --nocapture`（8 passed）。
