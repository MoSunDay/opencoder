Commit: 06c69a687d4e5b8416df9376a48f112bd8eb2618

# 节点对话侧栏一键删除全部会话（保留运行中）

## 交互

- Agent/Operator 对话页（SPA `chat`）右侧会话列表底部新增 danger 按钮「删除全部会话」：无选中节点或当前节点无会话时 disabled；点击弹 `Modal.confirm` 二次确认，文案明确「正在运行中的会话会保留」。
- 确认后 `DELETE /api/nodes/:id/dialogs` 批量清理：只删终态（done/error/cancelled）节点任务的 synthetic session，FK 级联带走 messages/events/inputs/subagent_tasks/node_tasks 行；pending/running/cancelling 的会话跳过并在响应 `skipped` 数组返回。
- 成功通知区分数量：「已删除 N 个会话」+（有跳过时）「，M 个运行中的会话已保留」；若当前选中会话被删则清空右侧转写并取消选中；删除后重载侧栏列表。取消确认不产生任何请求。

## 实现

- store：`ClearNodeDialogs { removed, skipped }`（`types.rs`）；`Store::clear_node_dialogs` trait（缺省 bail）；libsql `clear_finished_sessions` 在单条 `BEGIN IMMEDIATE` 事务内先查非 terminal 的 `session_id` 为 skipped，再 `DELETE FROM sessions WHERE id IN (terminal 子查询)`，级联与幂等由 SQL 保证。
- HTTP：`api_control::clear_dialogs`（未知节点 404，成功 `{ok, removed, skipped}`），路由 `/api/nodes/:id/dialogs` 挂 `delete`，与既有 `get(list_dialogs)` 共存。
- 前端：`chatSidebar.jsx` 底部按钮（新 prop `onDeleteAll`）；`chat.jsx` `deleteAllDialogs` 走确认→请求→重载→通知链。

## Validation

- store 单测：`crates/store/tests/node_dialogs_clear.rs`（新，3）：终态删/非终态留且 skipped 上报（5 任务 mixed + 邻节点隔离）、空节点与未知节点 no-op、级联删消息与任务行。
- web 集成：`crates/web/tests/node_dialogs_clear.rs`（新，2）：3 任务 running/pending/done sweep 断言 removed=1、skipped 含 running+pending、dialogs 索引只剩幸存者、级联删除、二次 sweep no-op；未知节点 404。
- SPA DOM：`crates/web/spa/src/chat/chatClearAll.dom.test.jsx`（新，3）：确认+跳过运行中+通知文案、选中会话被删后清除高亮、无节点/空节点禁用。antd 静态 Modal 关闭有退出动画，断言需等「新出现」的 `.ant-modal-confirm` 再取最后一个。
- 回归：`cargo test -p opencoder-store` 全绿；`cargo test -p opencoder-web` 65 个套件全绿；SPA 全量 vitest 112 文件 814 用例通过；`npm run build` 重建 `spa/dist`。

## Related Docs

- [web 模块](../../../agents/web/index.md)
- [store 模块](../../../agents/store/index.md)

## Release

- `974514b7` 首次发布（rel-974514b795d9e7b3d58cddf8e7986282f3991041）：三件套平滑上线，公共入口 18081 `/static/app.js` 与提交 dist 字节一致。上线后核验发现线上 405：生产路由表在 `crates/control` compat（dialogs 仅挂 `get`），web crate 的 DELETE 未接入生产 router。
- `d5a40572` 追发修复（rel-b16222f308c72d0ca890d7f8a0b2a4e5f7660f20）：compat `/api/nodes/:id/dialogs` 补挂 `.delete(sessions::clear_dialogs)`（未知节点 `RpcReply::error(404, "node not found")`，成功 `RpcReply::ok({ok, removed, skipped})`，与 web 版共用 `Store::clear_node_dialogs`）。control 集成测试 `crates/control/tests/node_dialogs_clear.rs`（新，2）：终态删/running 保留且 skipped 上报、未知节点 404。线上核验：DELETE 未知节点 404（不再 405），SPA 未变更（app.js sha 不变），新版本 host+runtime 服务 active。
- `78f62130` 重设计：生产数据在节点侧——`Store::clear_node_dialogs` 在生产拓扑必然 404（control.db `execution_index` 只有索引行，sessions 在各 runtime 的 runtime.db），control 版 store 删除全空。改为三层节点中继：
  - store：`Store::delete_sessions`（批删 sessions，FK 级联 messages）+ `FleetStore::delete_terminal_indexes`（按 node+kind+可删状态 `idle|done|error|cancelled` 删 `execution_index` 连带 assignments/receipts，`BEGIN IMMEDIATE` 单事务；状态门禁 8 态全列，e2e 曾抓到漏 `idle` 的 bug）。可删状态与 SPA 契约对齐：删 `{idle,done,error,cancelled}`，跳过 `{pending,running,cancelling,interrupted}`。
  - worker：Maintenance `dialogs_clear` 分支——活跃执行跳过（数据保留），其余 `delete_sessions` + `journal.forget`（防下一次全量 IndexReport 复活）。
  - control compat `clear_dialogs`：`fleet.nodes` 404 → `indexes(Operator)` 算 drop_ids → `hub.call(Maintenance dialogs_clear)` → `delete_terminal_indexes` → `{ok, removed, skipped}`。免 admission（admission 只匹配 ask|configure），故意不进 `maintenance_tools.rs` LLM 白名单。
  - 测试：`crates/control/tests/node_dialogs_clear.rs`（未知节点 404/离线 503 守卫）、`crates/control/tests/e2e/compat_nodes.rs::dialogs_delete_clears_node_and_terminal_indexes`（MockNode 全链路，idle 删/running 留）、`crates/worker/tests/maintenance_dialogs_clear.rs`（删除+遗忘、Hold ChatStream 控活跃跳过）、`crates/store/tests/delete_sessions_and_indexes.rs`（FK 级联/8 状态门禁/节点+kind 归属防串）。
- `026335f9` 复活根因根治：host `sync_inventory` 会把**所有** runtime（含休眠 runtime 的 saved final inventory）聚合上报，control 只删 control.db 行会在下一个上报周期被复活（实测 rel-a51016ca 休眠 inventory 存 17 条 operator 行）。host 端 `route()` 新增 `dialogs_clear` 分支：活跃 runtime 转发 Maintenance；休眠 runtime 从 `runtime_sleep` saved inventory 剔除可删行；聚合 `{ok, removed, skipped, forgotten}`。测试 `crates/agent/src/host/tests.rs::host_dialogs_clear_*`。
- 线上验证（human-os-02，rel-5e130f7f）：DELETE → `{"ok":true,"removed":18,"skipped":[]}`；control.db operator 行 18→0 且跨上报周期不复活；GET dialogs → `{"dialogs":[]}`；全部休眠 inventory 无 operator 残留；未知节点 404。SPA 未变更（manifest spa_sha256 不变）。
- 已知边界：旧版本 agent 节点（如 eval-diagnose-node，rel-653c7162，无 dialogs_clear 分支）DELETE 返回节点原样错误 `{"error":"unknown maintenance operation"}`——fail-closed：节点无法清 journal 时若 control 仍删索引行会复活，故不转译、不降级，升级节点 agent 即恢复。
