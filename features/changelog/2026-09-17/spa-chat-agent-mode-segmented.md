Commit: 272b2b13

# SPA 会话页「Operator / Agent 模式」切换，入口更名 Agent

nav「Agent」分类下的会话入口（page key 仍 `chat`，深链与 localStorage 兼容）menu label「Operator」→「Agent」；页内新增会话模式切换。Operator 模式创建链路现状不变，Agent 模式以 `kind:'agent'` 直接发起单 agent 会话执行，字段/行为与 DAG `StepKind::Agent` 对齐——后端执行面（control 入口与门禁、worker `how_append`/结构化输出、e2e）见 [control 定时调度器与 agent 会话执行面](control-cron-scheduler-agent-sessions.md)。

## 变更

- `nav.js`：`chat` 页 menu label 改「Agent」；page key、`ALL_PAGES`、`HEADERLESS_REASONS`、`oc_nav_page` 约定全部不动。
- `chat.jsx`：页头「会话模式」Segmented（Operator 模式 / Agent 模式），`useLocalStorage` 持久化 `oc_chat_mode`（缺省 operator，陌生/损坏值收敛回 Operator）。
- 创建分流（其余控制台能力——列表、消息流、act/plan、`@` agent 菜单、model/compact/fork/interrupt——完全复用）：
  - Operator 模式：`newId('operator')`、body 不带 `kind`，现状不变。
  - Agent 模式：`newId('agent')` + body `kind:'agent'` + `how_append`；「知识追加」弹窗按 UTF-8 字节校验 ≤ 8192，超限禁保存且不落 wire，可清空，非空才随创建提交；attempt key 计入模式，防跨模式复用 id。
- `chatSidebar.jsx`：新增 `nodeKind` prop，节点下拉按模式经 `canUseNode(nodes, id, kind)` 过滤。
- 文案：`operators/panel.jsx`、`operators/nodeTable.jsx`、`agentsConfig.jsx` 中指会话页的「Operator」改「Agent 页」，admin-only 节点总览页签改称「节点总览」（tab key 仍 `operator`）。执行类型 `KIND_LABELS['operator']`、wire 枚举值、brain `builtin-operator` 均不动。
- `dist/static/app.js` 随源码重建。

## 测试覆盖

| 功能 | 测试 | 结果 |
|------|------|------|
| 模式持久化/损坏值回退/两种模式请求体/知识追加字节预算/按模式节点过滤 | `src/chat/chatMode.dom.test.jsx`（新增 10 用例） | 全绿 |
| 导航更名与文案牵连 | `nav.test.js`、`app.dom.test.jsx`、`operators/panel.dom.test.jsx`、`chat.dom.test.jsx`、`chat/*.dom.test.jsx`、`fleet.dom.test.jsx` | 全绿 |
| SPA 全量回归 | vitest run | 113 文件 / 832 tests 全绿 |
| dist 漂移门 | scripts/check-spa-drift.sh | no drift |
