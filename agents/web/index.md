Commit: 5bf6f621e3722e60258592267109ba5807e74d94

# web 模块

axum HTTP/SSE 会话管理与编译期内嵌 SPA。

## 关键路径

- `src/lib.rs` — `AppState`（store/handles/nodes/controls/project/team/brain）与路由装配。
- `src/api.rs` — `/prompt` admit 即返回；draining 中 agent 切换 409。
- `src/api_events.rs` — `/events` SSE replay+live；`/api/sessions/:id/seq` 持久化事件游标。
- `src/handle.rs` — `SessionHandle` ring 缓冲+broadcast；`admit_and_drain_guarded`/`drain_to_completion`。
- `src/handle_lifecycle.rs` — `lock_session_lifecycle` 复核同 handle，防锁旧对象。
- `src/sse_dedup.rs` — `forward_live` live 去重与 pre-subscribe gap 桥接。
- `src/auth_mw.rs` — Bearer → Identity：seed token 常量时间比较或用户表 sha256 反查；`exempt` 豁免 `/`、`/static/*`、`/api/time` 等；control 经 `#[path]` 复用。
- `src/html.rs` — SPA 产物 `include_bytes!` 内嵌，`/`+`/static/:name` 白名单。
- `src/api_ops.rs`、`src/cmd.rs` — fork/compact/handoff/skill/config/bg；`DrainCmd` 通道。
- `src/api_agents.rs`、`src/api_agent_resources.rs`、`src/api_agent_nfs.rs` — 版本化 agent 面 + NFS 导出。
- `src/api_inputs.rs` — 输入列表/删除/reorder。
- `src/api_dag_wasm.rs`、`src/api_dag_wasm_nfs.rs`、`src/nfs_exports.rs` — DAG wasm 模块池 API + 命名多 NFS 导出（agents/dag-wasm 两路）。
- `src/nfs_exports.rs::status` — 异步获取生命周期锁后读取真实导出状态，调用方等待正在进行的启动/停止。
- `src/api_questions.rs`、`src/handle_questions.rs` — question answer/skip 闭环。
- `src/api_subagents.rs` — 子代理任务列表；`DELETE /api/sessions?keep=` clear-all。
- `src/api_brain.rs` — brain CRUD/search/dispatch（typed 错误映射）与剧本 CRUD（list/get/create/validate/delete，`validate_draft` 写库前 400）。
- `src/api_teams.rs`、`src/api_teams_topics.rs`、`src/team_state.rs`、`src/team_hub.rs` — 团队运行时与话题。
- `src/api_project*.rs` — project HTTP 适配；未 init 全部 503。
- `src/api_todo_*.rs`、`src/todo_hub.rs` — TODO 模板/环境/run 分发。
- `src/api_nodes*.rs`、`src/api_control.rs`、`src/nodes_state.rs`、`src/sse_nodes.rs` — 节点注册/心跳/claim/控制。
- `src/api_dag.rs`、`src/api_nodes_dag.rs`、`src/sse_dag.rs`、`src/dag_state.rs` — DAG CRUD/dispatch/claim/SSE。
- `spa/src/` — React18+antd SPA（vitest），产物提交于 `spa/dist`。
- `spa/src/agentsConfig.jsx`、`spa/src/agentDetail.jsx` — Agent 列表只显示身份与操作；编辑在右侧 75% 抽屉中加载详情，列表保持挂载，保存或激活后刷新列表。资源引用、Prompt 编辑与历史回滚在详情中维护。
- `spa/src/harness/agentFields.jsx` — Agent 详情中的执行方式与命名配置绑定；仅 PUT `harness` 或 `harness_profile`，不覆盖资源引用。
- `spa/src/harness/management.jsx`、`configuration.js` — 默认／命名 Codex 配置管理；读写均只投影 `model` 和 `envs`。配置读取失败时阻止保存，保存失败保留输入。
- `spa/src/harness/fields.jsx` — 启动字段；Codex 启动使用统一管理的 Wrap 参数。独立 Runner 管理入口不在 SPA 中，宿主机执行入口为 `operators/`。
- `spa/src/fleet/` — 节点/执行/团队/调度面板。
- `spa/src/brain/workbench/` — 能力/计划/运行工作台；图投影、原子快照水位与事件重连、步骤实例分页和检查面板。能力库页签直接是 `brainPanel.jsx` 能力 CRUD 表（行点击进 `brain/capabilityEditor.jsx` 抽屉），无成熟度列与 `+` 展开行；页自带 Tabs 标题，属 `nav.js` 的 `HEADERLESS_PAGES`，PageShell 只渲染无页头的 `.oc-page` body。
- `spa/src/fleet/detail.jsx` 的 ExecutionView — 四类过程的共享查询/渲染入口；受 Brain 管理的执行隐藏独立修改操作。
- `spa/src/brain/workbench/useRun.js` — 激活事件流结束后按同一运行 ID 重连，直到根运行进入终态；重连与快照刷新使用独立计时器。
- `spa/src/dag/run/` — 原生与嵌入页共用结果快照、状态增量与日志抽屉；`process.jsx` 只投影当前节点状态。快照水位过滤历史帧，完成事件先于账本落盘时同水位不回退运行态，更新水位可进入新尝试。
- `spa/src/ui/executionEvents/` — 日志按需批量加载历史后衔接实时流，保留有界窗口、分页、搜索和滚动；卸载终止请求，日志帧不驱动画布状态。
- `spa/src/envs/todoPanel.jsx` — TODO 模板环境（TODO env）与工具入口。
- `spa/src/project/` — 项目目标/里程碑/TODO 面板。
- `spa/src/todo/editor/`、`spa/src/todoEditor.jsx` — TODO 模板编辑器默认画布，表单/画布/JSON 共享 spec 草稿；保留 metadata、门禁与节点改名后的依赖引用，支持派发上下文预览。画布坐标仅会话态；宿主为 100% 宽右侧 Drawer，关闭脏草稿有确认提示。
- `spa/src/todoPanel.jsx` — 菜单页「TODO 管理」：模板 tab 的新建/编辑都走 100% 宽右侧 Drawer（列表保持挂载，关闭即 bump 刷新），运行 tab 是 `todoRunsPanel`。注意：抽屉展开后的 DOM 测试里全局 `getAllByRole` 会因 RTL `isInaccessible`→jsdom `getComputedStyle`（antd CSSINJS 大规则表）慢到分钟级，交互断言改用局部 `querySelectorAll`+文本归一化。
- `spa/src/todo/review/` — 工作流快照、任务筛选、候选/门禁/上下文与历史会话 Review；`useReview.js` 结合 generation、事件水位、SSE 和轮询，陈旧或失败时禁用控制；`api.js` 按 etag 拼装大字段。
- `spa/src/todo/runCanvas.jsx`、`spa/src/todo/runProjection.js` — 将 Review 状态投影到只读依赖图；选中任务联动 Inspector。完整执行详情复用工作台，inline 过程视图保留轻量内容。
- `spa/src/todo/review/rerun.jsx` — 展示目标和下游影响、要求原因；不确定回执重试保留 request_id，收到持久化 queued 回执才关闭。历史尝试选择固定其上下文，会话按消息游标增量读取。
- `spa/src/ui/tableLoading.js` — 列表表格 `loading` 的唯一约定：`tableLoading`（带 delay，裸 boolean 会变成 `delay:0` 闪遮罩）+ `tableRows`（拉取中交回 `undefined`，否则 antd 对着用户断言「暂无数据」）。新增表一律走它。
- `scripts/acceptance/spa_responsive.js` — 390×844 手机视口横向溢出门禁，服务**工作树** `spa/dist`；启动打印 bundle 溯源，`--require-committed` 拒测非 HEAD 产物、`--drift` 先跑漂移检查。
- `scripts/check-spa-drift.sh` — `spa/dist` ↔ `src` 漂移检查；压缩器对同一 src 偶发不同标识符命名（实测 4 次构建 1 次变体），故仅在差异局限于 `static/app.js` 时重建重试。
- `tests/` — HTTP/SSE 契约与节点 e2e 测试。

## 边界

- 无 LLM 单例：每 prompt 按配置构建；`client_override` 仅注入接缝。
- `opencoder-server` 走 control，不启动本模块执行面；worker 进程内复用 session router。

## 相关

- [brain](../brain/index.md) — 工作台数据契约；[业务交互](../../features/brain/index.md)。
- [agents/session](../session/index.md) — drain 与 cancel。
- [agents/store](../store/index.md) — 持久化与事件回放。
- [agents/control](../control/index.md) — 平台控制面。
- [agents/worker](../worker/index.md) — 节点执行面。
- [TODO 工作流](../../features/todos/index.md) — 画布、Review 与节点重跑规则。
- [Agent Harness](../../features/harness/index.md) — 执行方式、Wrap 参数与配置快照规则。
