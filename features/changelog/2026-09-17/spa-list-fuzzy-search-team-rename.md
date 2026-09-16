Commit: 134deab2

# TODO/DAG/Team 列表模糊搜索 + 「团队」更名 Team

对齐 Agent 列表（`agentsConfig.jsx` AgentListPanel）的既有交互：受控 `Input.Search`（allowClear、minWidth 220、aria-label、占位文案声明搜索范围），`search.trim().toLowerCase()` 对本地列表做忽略大小写子串过滤，纯本地、不入服务端；空查询即全量。

## 变更

- 搜索框（过滤字段 → aria-label）：
  - Team 页 `fleet/teams.jsx`：name → `team-search`（「搜索 Team 名称」）。
  - DAG「定义」`dag/defsTab.jsx`：name、id → `dag-def-search`。
  - DAG「运行」`dag/runsTable.jsx`：id/name/status/node_id → `dag-run-search`。
  - TODO「模板」`todoPanel.jsx` TemplatesTab：name/description → `todo-template-search`。
  - TODO「运行」`todoRunsPanel.jsx`：id/status/execution_status → `todo-run-search`（置于「工作流」Card extra，与刷新按钮同组）。
- 「团队」→「Team」更名（用户可见文案）：侧栏菜单 `团队组队`→`Team 组队`（`nav.js`）；Team 页全部文案（创建 Team / Team 成员 / Team 名称 / 保存 Team / 启动 Team / 「整个 Team 会在同一个执行节点内完成」）；执行详情 `Team 执行`/`Team 协作内容`；项目页执行器选项与 Tag label `团队`→`Team`。wire 值（`team`）、`agents/team` 运行时语义不变；`teamModals.jsx` 为无生产引用的死代码，未动。

## Validation

- SPA 全量 vitest：113 文件 832 用例通过；新增 5 例搜索行为用例（`team.dom.test.jsx` 大写输入命中忽略大小写 + 清空恢复、`dag/dag.dom.test.jsx` 定义/运行两例、`todoPanel.dom.test.jsx` 模板一例、`todoRunsPanel.dom.test.jsx` 运行一例，后者沿用其假定时器纪律）；`nav.test.js`/`fleet.dom.test.jsx`/`todosTab.dom.test.jsx` 断言同步更名。
- `npm run build` 重建 dist（编译期嵌入 `crates/web/src/html.rs` 的产物随之更新），`scripts/check-spa-drift.sh` 无漂移。

## Related Docs

- [web 模块](../../../agents/web/index.md)
