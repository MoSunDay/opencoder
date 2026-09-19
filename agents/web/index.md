Commit: 35377f739c15039b725df8c135a9a14cec8bf815

# web 模块

axum HTTP + SSE 会话管理 + 内嵌 SPA。

## 索引
- `src/lib.rs` — `AppState` 装配
- `src/api.rs`、`src/api_*.rs` — 各域 HTTP API（prompt/events/agents/dag/todo/team/…）
- `src/api_agents.rs` — agent 目录 API：列表项含 `description`
  （soul 首行，回退 "Custom agent <name>"）与 `primary`
  （`is_primary() && name != "workflow"`，与 TUI 选择器同规则）
- `src/api_agent_resources.rs` — agent 资源文件 API：`safe_rel_path` 门
  （拒绝绝对/`..`/`.`/空段/隐藏点前缀段/64 段超深，先于任何 fs 工作），
  memory 目录化写侧，`section_body` 读侧降级见 core `agent/memory.rs`
- `src/handle.rs` — `SessionHandle` ring 缓冲 + broadcast
- `src/auth_mw.rs` — Bearer → Identity
- `src/html.rs` — SPA 产物内嵌；`/static/:name` 白名单 app.js/app.css/download-sw.js/favicon.png（tab 图标，`logo/logo.png` 64×64 派生，shell `link rel=icon` 引用，白名单契约测试双向锁定）
- `spa/src/` — React18+antd SPA（vitest），产物提交于 `spa/dist`；
  导航三分类（项目/Agent/节点）的最后选择经 usehooks-ts `useLocalStorage`
  镜像到 localStorage（`oc_nav_page`，键与校验集 `ALL_PAGES` 在 `nav.js`），
  重挂载时 `useLayoutEffect` 首帧前恢复，brain_run 深链优先；
  composer 命令菜单 `commandMenu.js` + `fuzzy.js`（与 TUI `/agent`、`@`
  agent 菜单同 fuzzy 语义；`@` sigil 条目只来自 agent 目录）
- `src/api_control.rs` — 节点对话 API：`GET/DELETE /api/nodes/:id/dialogs`。
  DELETE 一键清空该节点对话：仅删终态（done/error/cancelled）节点任务的
  synthetic session（FK 级联消息/任务行），pending/running/cancelling 保留
  并入响应 `skipped`；未知节点 404
- chat 页（nav menu「Agent」，原 Operator，page key 仍 `chat`）创建链路双模式：
  页头「模式 Segmented」（Operator 模式 / Agent 模式）经 usehooks-ts
  `useLocalStorage` 持久化（`oc_chat_mode`，缺省 `'operator'`，陌生/损坏值收敛
  回 Operator 展示与行为）；Operator 模式维持现状（`newId('operator')`，body
  不带 `kind`），Agent 模式创建走 `newId('agent')` + body `kind:'agent'` + 首条
  `prompt`；页面必须选择具体可执行 primary Agent，worker 在建本地 session 后将
  首条需求作为 how 追加并在成功后持久化（显式 `how_append` 仍受 8192 字节预算约束）；
  首条需求随创建请求一次提交（契约由 operator_e2e O5 锁定；旧 create→seq→prompt 三步链
  会与节点异步启动竞态，首次提交偶发 404），SSE 游标直接用 0 回放全会话；Agent
  与 Operator 共用 Say/transcript 输出，便于在 Web 调试具体执行能力；节点下拉
  与可执行判定按 `canUseNode(nodes, id, kind)` 分模式取值（chatSidebar 收
  `nodeKind` prop）；无独立「新建对话」按钮——选中节点首次发送即建会话，
  creation 入口已移除（空态文案引导直接输入）；其余能力（会话列表、消息流、act/plan、@ 菜单、model/
  compact/fork/interrupt）模式无关复用。agentsConfig admin-only 页签 label
  「Operator」→「节点总览」（tab key 仍 `operator`）；执行类型 `KIND_LABELS
  ['operator']` 等 wire 值一律不变
- chat 页白底面板停靠：SHEET_PAGES（main.jsx）页面的 Content 挂
  `fleet-content--flush`（去底部 padding）+ `.fleet-sheet` 方角贴住视口底边
- `spa/src/fleet/` — 全部执行表含「名称」列（列表端点提升的派发时快照，缺省渲染 `-`）；详情抽屉标题「名称 (id)」，缺名称回退裸 id
- `spa/src/` 列表模糊搜索：Team / DAG 定义 / DAG 运行 / TODO 模板 / TODO 运行五处列表挂与 Agent 列表同语义的受控 `Input.Search`（本地忽略大小写子串过滤、
  空查询即全量、不入服务端；aria-label `team-search` / `dag-def-search` / `dag-run-search` /
  `todo-template-search` / `todo-run-search`）；Team 页用户文案「团队」→「Team」
  （nav menu「Team 组队」，page key 与 wire 值 `team` 不变）
- `spa/src/agents/` — Agent 配置资源页签（prompts/skills/tools/memory）：唯一工具行
  （版本、保存、覆盖上传、未保存/已保存、Prompt 预览开关）经 antd `Space` 统一间距；
  Skills/Memory/Tools 的结构操作只走 FileWorkspace 树右键（新增/重命名内联草稿、
  删除确认），无单文件上传按钮；「上传压缩包」`archiveUpload.jsx` 只接受 .zip，
  前端解包（fflate）→ `mergeArchive` 覆盖语义合并进草稿（同名文件覆盖并沿用权限位、
  目录内其他文件保留；skills 校验合并集含 SKILL.md；≤1.5 MiB/4096 文件，与后端
  `merge_files` 同规），仍由「保存」走 files-only PUT，后端契约不变；
  `useResources.js` 持有 entries/edit/save/refresh（uploading 链路已删）

## 接缝
- 会话执行复用 session 运行时；持久化经 `Arc<dyn Store>`。
- `spa/src/schedule/panel.jsx` —「定时任务」页（Agent 分类，page key
  `schedules`，menu-only）：`GET /api/schedules` 定义列表（id/cron/enabled/
  kind/target/overlap/node_id/last_run/next_run，invalid cron 的 next_run
  渲染「—」）+ `GET /api/schedules/:id/runs?limit=50` 触发历史 Drawer
  （scheduled_for_ms/fired_at_ms/status/execution_id/error）；只读（事实源
  schedules.json），5s 静默轮询；admin-only 端点与非 admin 导航裁剪天然对齐
- `spa/src/brain/workbench/` 编辑 v2 的 input、实例、output、路由，能力选择使用注册目录；`editor/` 管理端口、映射、出口与草稿。提交具名文档后，画布和 inspector 按 visit、output、route 展示活跃分支、等待原因、完成／验证证据和局部路由读集。
- Brain 的校验、快照发布和执行由 [control](../control/index.md) 与 [brain](../brain/index.md) 提供；前端不另行推断路由或验证结果。历史旧格式只读，新写入提示迁移。
