Commit: 7e71cbcfd669dd2cbaa5c94ab01945fd139557f0

# web 模块

axum HTTP + SSE 会话管理 + 内嵌 SPA。

## 索引
- `src/lib.rs` — `AppState` 装配
- `src/api.rs`、`src/api_*.rs` — 各域 HTTP API（prompt/events/agents/dag/todo/team/…）
- `src/api_agents.rs` — agent 目录 API：列表项含 `description`
  （soul 首行，回退 "Custom agent <name>"）与 `primary`
  （`is_primary() && name != "workflow"`，与 TUI 选择器同规则）；
  卡片 `run_mode` 全链路透传——POST/PUT body 接受（POST 缺省 `operator`，
  PUT 缺省不动、变更追加 `run_mode` history 条目），列表项与
  `/api/agents/:name/meta` 均暴露；非法值走 axum Json 数据拒绝 422
  （与 `harness` 同路径，web_agents.rs 锁定）
- `src/api_agent_resources.rs` — agent 资源文件 API：`safe_rel_path` 门
  （拒绝绝对/`..`/`.`/空段/隐藏点前缀段/64 段超深，先于任何 fs 工作），
  memory 目录化写侧，`section_body` 读侧降级见 core `agent/memory.rs`
- `src/handle.rs` — `SessionHandle` ring 缓冲 + broadcast；`src/handle/drain.rs` 管理 drain 生命周期，以 `DrainContext` 携带执行目录和配置。
- `spa/src/dag/dynamic/` — 来源及模板表单、文本/argv 派发批次、实例分页与单实例订阅；切换时清理旧流，列表与详情错误分别保留。模板终态通过回执刷新收敛，进度按绝对计数与快照时间归并。见 [动态 DAG 步骤](../../docs/dag-dynamic.md)。
- `src/auth_mw.rs` — Bearer → Identity
- `src/html.rs` — SPA 产物内嵌；`/static/:name` 白名单 app.js/app.css/download-sw.js/favicon.png（tab 图标，`logo/logo.png` 64×64 派生，shell `link rel=icon` 引用，白名单契约测试双向锁定）
- `spa/src/` — React18+antd SPA（vitest），产物提交于 `spa/dist`；
  导航三分类（项目/Agent/节点）的最后选择经 usehooks-ts `useLocalStorage`
  镜像到 localStorage（`oc_nav_page`，键与校验集 `ALL_PAGES` 在 `nav.js`），
  重挂载时 `useLayoutEffect` 首帧前恢复，brain_run 深链优先；
  composer 命令菜单 `commandMenu.js` + `fuzzy.js`（与 TUI `/agent`、`@`
  agent 菜单同 fuzzy 语义；`@` sigil 条目只来自 agent 目录，候选集按模式
  取值——Operator 模式内置在前+注册卡、Agent 模式仅注册卡：内置是 loop 的
  runtime 执行角色、注册 Agent 卡是另一类物，merge 仅限调度执行器目标与
  Operator 切换面两类用途，分界唯一事实源为 `spa/src/agents/builtins.js`
  头注释；会话切换端点 `POST /api/sessions/:id/agent` 同样排除 workflow）
- `src/api_control.rs` — 节点对话 API：`GET/DELETE /api/nodes/:id/dialogs`。
  `?kind=operator|agent` 选择独立对话 lane（缺省 operator）；DELETE 只清理
  所选 lane 的终态（done/error/cancelled）节点任务 synthetic session（FK 级联
  消息/任务行），pending/running/cancelling 保留并入响应 `skipped`；未知节点 404
- chat 页（nav menu「Agent」，原 Operator，page key 仍 `chat`）创建链路双模式：
  页头「模式 Segmented」（Operator 模式 / Agent 模式）经 usehooks-ts
  `useLocalStorage` 持久化（`oc_chat_mode`，缺省 `'operator'`，陌生/损坏值收敛
  回 Operator 展示与行为）；Operator 模式维持现状（`newId('operator')`，body
  不带 `kind`），Agent 模式创建走 `newId('agent')` + body `kind:'agent'` + 首条
  `prompt`；页面必须选择具体可执行 primary Agent——「执行 Agent」下拉的候选集
  只来自 Agent 配置（`GET /api/agents`）的 primary 注册卡，内置 act/plan/command
  是 operator 宿主循环的角色、不进入该泳道（`mergeBuiltinPrimaryAgentCards`
  合并只服务 Operator 模式的 `@` 菜单与选中兜底；Agent 模式的 `@` 菜单与下拉
  同源只列注册卡），默认收敛第一张注册卡，配置为空时发送被门禁拦截并提示先去
  「Agent 配置」创建，worker 在建本地 session 后将
  首条需求作为 how 追加并在成功后持久化（显式 `how_append` 仍受 8192 字节预算约束）；
  首条需求随创建请求一次提交（契约由 operator_e2e O5 锁定；旧 create→seq→prompt 三步链
  会与节点异步启动竞态，首次提交偶发 404），SSE 游标直接用 0 回放全会话；Agent
  与 Operator 的列表/删除 lane 分开但共用 Say/transcript 输出，每行带 `kind` 与
  `execution_ref`，Agent 明细仍定位到 Operator-capable 节点；节点下拉
  与可执行判定按 `canUseNode(nodes, id, kind)` 分模式取值（chatSidebar 收
  `nodeKind` prop）；无独立「新建对话」按钮——选中节点首次发送即建会话，
  creation 入口已移除（空态文案引导直接输入）；其余能力（会话列表、消息流、Operator 模式
  的 act/plan Segmented、model/compact/fork/interrupt）模式无关复用，`@` 菜单
  候选集按模式取值（见上）。agentsConfig admin-only 页签 label
  「Operator」→「节点总览」（tab key 仍 `operator`）；执行类型 `KIND_LABELS
  ['operator']` 等 wire 值一律不变
- chat 页白底面板停靠：SHEET_PAGES（main.jsx）页面的 Content 挂
  `fleet-content--flush`（去底部 padding）+ `.fleet-sheet` 方角贴住视口底边
- Agent 卡片 `run_mode`（`operator`=节点宿主机进程（现状默认） / `agent`=每轮
  runc 只读沙箱，缺失/陌生值一律收敛 operator）：SPA 侧唯一事实源
  `spa/src/agents/runMode.js`（Segmented options、一行说明、徽标文案、
  `normalizeRunMode`）；新建 Agent modal（agentsConfig）「运行模式」Segmented
  默认 operator，POST `/api/agents` body 恒带 `run_mode`；详情编辑面
  （`harness/agentFields.jsx`）同款 Segmented 绑定 `meta.run_mode`，改动即
  PUT `/api/agents/:name` 仅携带 `{run_mode}`；详情 Meta tab 顶部 Tag
  （`operator · 宿主机` / `agent · 沙箱`）+ 同一行说明；chat 页 Agent 模式
  选中具体 Agent（仅注册卡）时在描述旁挂「宿主机/沙箱」小 Tag（tooltip 同
  说明，缺失 `run_mode` 的注册卡按宿主机展示）。会话创建请求不带 `run_mode`——
  worker 从目标 Agent 卡片读取
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
- `spa/src/schedule/panel.jsx` + `editor.jsx` —「定时任务」页（Agent 分
  类，page key `schedules`，menu-only）：`GET /api/schedules` 定义列表
  （id/cron/enabled/kind/target/overlap/node_id/last_run/next_run，invalid
  cron 的 next_run 渲染「—」）+ `GET /api/schedules/:id/runs?limit=50` 触发
  历史 Drawer + 新建/编辑/启停/删除/立即触发的 admin CRUD；5s 静默轮询。
  新建表单隐藏 ID（后端生成 `schedule-<ULID>`）与时区（固定 `+08:00`），
  编辑保留两字段；kind 收敛为后端五种；params 按 kind 单键文本分流
  （agent/team/todos→`prompt`、dag→`args`、brain→`objective` 必填），编辑
  合并 `initial.params` 其余键、agent 清 `how_append` 旧键；页面不再渲染
  种子语义提示
- `spa/src/brain/workbench/` 编辑 v2 的 input、实例、output、路由，能力选择使用注册目录；`editor/` 管理端口、映射、出口与草稿。提交具名文档后，画布和 inspector 按 visit、output、route 展示活跃分支、等待原因、完成／验证证据和局部路由读集。v3 运行工作台只画目标、阶段、当前轮次和结果四个摘要节点，轮次按当前展开、历史折叠展示能力 operation 索引；点击 execution ID 复用 `ExecutionView` 托管抽屉读取节点明细，DAG 的步骤日志继续由原执行组件实时读取。
- Brain 的校验、快照发布和执行由 [control](../control/index.md) 与 [brain](../brain/index.md) 提供；前端不另行推断路由或验证结果。历史旧格式只读，新写入提示迁移。
