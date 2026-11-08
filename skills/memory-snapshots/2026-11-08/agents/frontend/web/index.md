Commit: 5afb497be53de0f763f06c26a1eb57709198874d

# web 模块

axum HTTP + SSE 会话管理与内嵌 React SPA。

## 索引

- `src/lib.rs` — `AppState` 装配与 `serve` 入口（Operator `config_home`、drain 栈）
- `src/api.rs`、`src/api_*.rs` — 各域 HTTP API
  （prompt/events/agents/dag/todo/team/…）
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
- `admin/` — 后台管理页（用户、节点）
- [login.jsx](../../../crates/web/spa/src/login.jsx)、`main.jsx`) — 登录与身份确认挂载
- [chat.jsx](../../../crates/web/spa/src/chat.jsx)、`chat/`、
  [conversationInput.jsx](../../../crates/web/spa/src/chat/conversationInput.jsx) —
  会话页与共享输入组件
- [chat/useTranscriptStream.js](../../../crates/web/spa/src/chat/useTranscriptStream.js) —
  流结束后合并执行结果与会话快照
- [stepsBlock.jsx](../../../crates/web/spa/src/stepsBlock.jsx) — Step 阶梯
  三级展开：纯用户控制、默认折叠（L0–L3 全部非受控）；运行中
  （`ladderRunning`，Say 之前）的最后一步由 **`TurnLiveRegion` 固定面板**
  流式渲染（头部仅 `⏺ live`，主体渲染 thinking + 与阶梯 L3 同款的
  `ToolContent` 调用行（input/result 均收在默认关闭的折叠内）+
  流式 Say `💬 Say` 块（`say` prop = `state.pendingSay`，text_delta 只
  累积不动 turns，flush 边界才落地），无
  Step(k) 计数——计数归消息列表阶梯）——面板在滚动区外
  （transcript 滚动 div 与 QueuePanel 之间），绝不出现在气泡内；
  `findRunningStepsTurn` 从最新 turn 往回找 running steps turn（最新已
  settle 即无），settle 后实时区清空；与 TUI `chat_live_turn.rs` 同契约
- [chat/inputAttempt.js](../../../crates/web/spa/src/chat/inputAttempt.js) — 人工输入 ID
  的生成与重试规则
- [nav.js](../../../crates/web/spa/src/nav.js)、
  [shell/categoryTabs.jsx](../../../crates/web/spa/src/shell/categoryTabs.jsx) —
  平台导航分类
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
- [theme.js](../../../crates/web/spa/src/theme.js) — antd 主题配置
- `harness/` — Codex 默认配置与档案编辑
- [Ontology 前端](ontology.md) — 五页面前端（图谱、实体、类型、关系与
  数据保鲜，面板挂载见 `spa/src/ontology/panels.tsx`）；实体页目录
  Tabs（目录树聚焦与成员管理）详见同目录 [ontology.md](ontology.md)；
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
