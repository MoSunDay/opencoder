Commit: d0e87cbca

# web 模块

axum HTTP + SSE 会话管理与内嵌 React SPA。

## 索引

- `src/lib.rs` — `AppState` 装配与 `serve` 入口（Operator `config_home`、drain 栈）
- `src/api.rs`、`src/api_*.rs` — 各域 HTTP API
  （prompt/events/agents/dag/todo/team/…）；`map_broadcast_result`/`lag_payload`
  为 SSE 广播 lag 帧的共享 wire 契约（`error` + 数值 `lag` 标记）
- [sse_dag.rs](../../../crates/web/src/sse_dag.rs) — DAG 运行事件流（回放+直播、
  Last-Event-ID 续传、exact-seq 去重）；直播 `Lagged` 合成带 `lag` 标记的
  `error` 帧（id 复用回放头作重连游标），不再静默丢弃
- [api_transcript.rs](../../../crates/web/src/api_transcript.rs) — 会话完整展示消息块
  按序号与字节偏移读取
- `src/api_agents.rs`、`src/api_agent_resources.rs` — agent 目录/卡片与资源文件 API
- `src/api_dag_binaries*.rs`、`src/api_dag_workspace_nfs.rs` — 二进制版本池与
  只读 NFS 导出
- `src/api_project*.rs` — 项目、子项、TODO、Tag 共享处理器（Control 复用）
- [api_project_people.rs](../../../crates/web/src/api_project_people.rs) — 用户名目录
- [handle.rs](../../../crates/web/src/handle.rs)、`src/handle/`) — `SessionHandle` 与
  drain 生命周期
- [handle_lifecycle.rs](../../../crates/web/src/handle_lifecycle.rs) — SSE 订阅清零后的
  handle 回收
- `src/auth_mw.rs` — Bearer → Identity；`src/html.rs` 编译期内嵌 `spa/dist`
- `spa/src/` — React SPA（vitest）；修改页面后须重建产物
- [check-spa-drift.sh](../../../scripts/check-spa-drift.sh) — SPA 产物漂移检查
- [access/useServerAuth.js](../../../crates/web/spa/src/access/useServerAuth.js) —
  公开鉴权状态读取
- `admin/` — 后台管理页（用户；节点面板在 `fleet/nodes.jsx`）
- [login.jsx](../../../crates/web/spa/src/login.jsx)、`main.jsx`) — 登录与身份确认挂载
- [chat.jsx](../../../crates/web/spa/src/chat.jsx)、`chat/`、
  [conversationInput.jsx](../../../crates/web/spa/src/chat/conversationInput.jsx) —
  会话页与共享输入组件
- [chat/useTranscriptStream.js](../../../crates/web/spa/src/chat/useTranscriptStream.js) —
  流结束后合并执行结果与会话快照
- [stepsBlock.jsx](../../../crates/web/spa/src/stepsBlock.jsx) — Step 阶梯
  三级展开：纯用户控制、默认折叠（L0–L3 全部非受控）；运行中
  （`ladderRunning`，Say 之前）的最后一步由 **`TurnLiveRegion`** 流式
  渲染（头部 `> streaming` 展开 / `▸ streaming` 收起，dim warn；
  默认展开，点击头行或 Ctrl/Cmd+L 收起，每个新 turn 重新展开；折叠仅
  用 `display: none` 隐藏主体，保留嵌套 `ToolContent` 的开合状态。主体
  仅 thinking + 与阶梯 L3 同款的 `ToolContent` 调用行，无 Step(k) 计数、
  无 Say 块）——面板在
  transcript 滚动容器**内**、消息列表之下，与消息共用滚动条；流式
  Say 由 `StreamingSay`（`state.pendingSay` 原文、无标记）渲染在同一
  滚动流的列表与面板之间，flush 边界落地为消息（text_delta 只累积
  不动 turns）；auto-stick 仅在贴近底部时跟随（手动上滚优先）；
  `findRunningStepsTurn` 从最新 turn 往回找 running steps turn（最新已
  settle 即无），settle 后实时区清空；与 TUI `chat_live_turn.rs` 同契约；夹在
  阶梯与其 Say 之间的 subagent 块对配对透明：`bubbleItems.js::itemsFromTurns`
  把 run 内 `subagent` 部分与 `sys` 同样吸收进同一气泡（保持块序），
  `transcript.jsx::AssistantTurnContent` 把该段渲染在头行**上方**（不切入 Say：
  头行与正文连续，见
  [changelog](../../../features/changelog/2026-11-27/subagent-section-above-say.md)）
- [subagentFold.js](../../../crates/web/spa/src/subagentFold.js)、
  [subagentBlock.jsx](../../../crates/web/spa/src/subagentBlock.jsx) — subagent 块的
  转录级统一披露（TUI `toggle_subagents` 同契约）：`TranscriptView` 持有唯一的
  `{ open, toggle }`（默认收起）经 `SubagentFoldContext` 下发，每个 subagent 头部显示
  阶梯同款 `❯`/`▸`（`stepsBlock.jsx::disclosureIcon`），点击任一头部即同时开 / 合全部
  subagent 块（`[→ view]` 阻止冒泡，仍只打开回放弹窗）；Ctrl/Cmd+L 与 `⤒ 收起` 走同一
  `collapseAll`：bump epoch 重挂气泡并把 subagent 披露置为收起（见
  [changelog](../../../features/changelog/2026-11-27/subagent-fold-toggle.md)）
- [chat/inputAttempt.js](../../../crates/web/spa/src/chat/inputAttempt.js) — 人工输入 ID
  的生成与重试规则
- [nav.js](../../../crates/web/spa/src/nav.js)、
  [shell/brandNav.jsx](../../../crates/web/spa/src/shell/brandNav.jsx)、
  [shell/categoryTabs.jsx](../../../crates/web/spa/src/shell/categoryTabs.jsx) —
  平台导航分类；头部品牌菜单只列一级导航（tab），每个分类一项（图标 + 分类名，
  hover/click），选择后进入该分类首页；Sider 只保留当前分类 Menu，移动端为
  分类 tabs + Select
- `project/` — 项目/子项/TODO/Tag 表格、表单、看板与评论
- [project/navigation/links.js](../../../crates/web/spa/src/project/navigation/links.js)
  — `project/initiative/todo` 定位编解码
- `project/summary/` — 现状展示、历史与恢复
- [todoDrawer.jsx](../../../crates/web/spa/src/project/todoDrawer.jsx)、
  `project/execute/` — TODO 能力派发与关联
- `fleet/`、`schedule/` — 执行表、节点与定时任务页
- [fleet/navigation/](../../../crates/web/spa/src/fleet/navigation/links.js) — 执行深链
  定位与历史
- [ui/requests/query.js](../../../crates/web/spa/src/ui/requests/query.js) — 读取请求
  取消、迟到响应丢弃与错误状态
- [ui/tableLoading.js](../../../crates/web/spa/src/ui/tableLoading.js)、
  [ui/usePoll.js](../../../crates/web/spa/src/ui/usePoll.js) — 表格 loading 约定与
  可见性感知轮询
- `ui/runContent/` — 安全 Markdown、字段树与产物下载
- `agents/`、[agentDetail.jsx](../../../crates/web/spa/src/agentDetail.jsx) — Agent 资源
  编辑入口
- `dag/` — DAG 编辑器、运行页与动态步骤
- `brain/workbench/` — Brain 计划编辑与运行工作台
- [brainPanel.jsx](../../../crates/web/spa/src/brainPanel.jsx) — 能力目录管理
- [theme.js](../../../crates/web/spa/src/theme.js) — antd 主题配置与 `--oc-*` 设计 token 的
  唯一色源（`palette` / `tints` / `SANS` / `FONT_SIZE`：深色画布 `#0d1117`、面板 `#161b22`、
  主色 `#58a6ff`、状态色、13px 基准字号、48px 头部、16px 内容间距、6px 主圆角、面板无阴影，
  只有浮层带一层深色投影）；
  [ui/mono.js](../../../crates/web/spa/src/ui/mono.js) — 唯一等宽字体栈（Latin 等宽在前、
  CJK 回退在后，与 `--oc-mono` 同序，组件经 `MONO_VAR` 引用）
- [app.css](../../../crates/web/spa/src/app.css) — 基础层（滚动条 / 焦点 / 内容面 / 浮层边框 /
  抽屉 / Markdown 覆盖 / 表格兜底），为遵守行数限制按职责拆分并在文件头 `@import`：
  [styles/tokens.css](../../../crates/web/spa/src/styles/tokens.css)（`--oc-*` token，
  `color-scheme: dark`）、[styles/shell.css](../../../crates/web/spa/src/styles/shell.css)
  （头部 / 品牌 / Sider / 统一页头 / sheet 与外壳响应式规则）、
  [styles/canvas.css](../../../crates/web/spa/src/styles/canvas.css)（DAG 节点与编辑器、
  React Flow / CodeMirror 外观）；主题配置 ↔ token 的锁步由 `theme.test.js`、
  `ui/mono.test.js` 守护；组件局部样式只引用 token、不再内联颜色字面量
  （`brain/workbench/`、`dag/run/`、`project/`、`todo/review/`、`ontology/`、
  `ui/files/`、`ui/runContent/`），接口 / 状态 / 事件 / 流程与响应式契约不变；实体详情的
  HTML 预览 iframe（[TextPreview.tsx](../../../crates/web/spa/src/ontology/pages/entityTypes/details/TextPreview.tsx)）
  是独立文档、`--oc-*` 不跨文档边界，故其 `srcDoc` 以同一 `palette` / `SANS` / `MONO` 派生的
  iframe 内 token 声明深色 body / code / mark / link 面与 `color-scheme: dark`（sandbox / CSP /
  data 行为不变），
  [content.css](../../../crates/web/spa/src/ontology/pages/entityTypes/details/content.css)
  的 iframe 外框走同一画布 token，`agents/archiveUpload.jsx` 路径列表走 `--oc-fill-subtle` /
  `--oc-border`；守护测试 [TextPreview.test.tsx](../../../crates/web/spa/src/ontology/pages/entityTypes/details/TextPreview.test.tsx)、
  [content.test.js](../../../crates/web/spa/src/ontology/pages/entityTypes/details/content.test.js)、
  [archiveUpload.dom.test.jsx](../../../crates/web/spa/src/agents/archiveUpload.dom.test.jsx)；见
  [changelog](../../../features/changelog/2026-11-27/server-web-dark-theme.md)
- `harness/` — Codex 默认配置与档案编辑
- [Ontology 前端](ontology.md) — 五页面前端（图谱、实体、类型、关系与
  数据保鲜，面板挂载见 `spa/src/ontology/panels.tsx`）；
  接口与存储见 [ontology](../../ontology/index.md)
- 验收入口：[ui/main.js](../../../scripts/acceptance/ui/main.js)、
  [project/](../../../scripts/acceptance/project/README.md)、
  [brain/ui/](../../../scripts/acceptance/brain/ui/history.js)、
  [conversations](../../../scripts/acceptance/conversations/README.md)

## 相关

- [control](../../platform/control/index.md)、[brain](../../orchestration/brain/index.md) —
  Brain 校验/快照发布/执行在后端
- [动态 DAG 步骤](../../../crates/dag/src/dynamic.rs)
- [DAG 能力](../../../features/platform/dag/index.md)、
  [执行约定](../../../rules/04-dag-execution-contract.md)
- [UI 验收约定](../../../rules/05-ui-acceptance.md)
