Commit: 2fae46b5

# 全部执行页改「筛选 + 表格 + 明细」，brain/todos 过程视图入明细抽屉

- 「Agent → 全部执行」回到浏览页语义：工具栏 = 唯一的 `执行类型筛选` Select + 刷新 + `启动执行` 按钮，内联启动表单原样搬进 `ExecutionLaunchModal`（`fleet/executionLaunch.jsx`，幂等 attempt/节点候选/Harness 字段逐字等价，成功即关窗开明细抽屉）——非 admin 只有这一页，启动入口保留但不再占据浏览首屏。
- `KINDS` 补 `maintenance → 维护执行`：节点维护产生的真实 kind 在类型列显示中文标签且可筛选（仍不可手动启动，CREATABLE_KINDS 不变）。
- 执行明细补齐两类过程视图：`brain` 内嵌工作台 `BrainRunBody`（run.jsx 拆出 Body/View，抽屉复用步骤列表 + PlanCanvas + Inspector + 调度事件 Timeline，**不写** `brain_run` URL 参数）；`todos` 内嵌 `TodoRunEmbed`（拉 `/api/todo/workflows/:id`，复用 runProjection 折叠 + TodoRunCanvas/TodoRunInspector + SSE 实时帧，404/分体部署拉取失败静默返回 null，TodoDetail 列表仍在下方回退，零回归）。
- 删除合并遗留死代码 `fleet/brain.jsx` 的 `BrainDispatch`（main 实挂 BrainWorkbench，且其期望的 `result.execution.id` 与现 `/api/brain/dispatch` 返回形状不符）。
- 修复 `scripts/check-spa-drift.sh`：HEAD 里 `workbench/editor/model.js` 以仓库相对深度 import `examples/brain/repair-loop.json`，旧临时树（裸 `spa/`）解析失败导致门禁误报 harness error；临时树改为镜像 `crates/web/spa` + `examples/` 布局。
- 验收脚本（platform / harness/codex / t12_ui_verify）适配 Modal：先点工具栏 `启动执行` 开窗，提交限定 dialog 作用域，`chooseInForm` 在 dialog 内取 combobox。
- 评审跟进（P2）：过程视图挂载加 `mode === 'full'` 门控——brain 工作台 Inspector「执行过程」页以 inline 复用 `ExecutionView`，而 brain 能力含 todos，否则 todos 实例会在 ~380px 窄列里嵌 TODO 画布 + 第二条 SSE；门控后 inline 只保留轻量块（Transcript/WorkloadDetail/TodoDetail），full（明细抽屉）行为不变。`embeds.dom.test.jsx` 补 inline/full 双态断言（inline 连画布数据都不拉）。
- 评审跟进（P3 清理）：`BrainRunView` 收敛为单层 `.brain-run`——返回栏/面包屑经 `header` 槽渲染进 Body 的唯一容器（旧双层嵌套是 Body/View 拆分残留，flex/gap 逐层等价但 DOM 冗余）；新增 `workbench/tests/run.dom.test.jsx` 锁单层容器 + `brain_run` 参数只由 View 写 + 返回回调。
- 备查（P3，与 todoRunsPanel 行为一致，不改）：`TodoRunEmbed` 在「fetch 成功但 SSE 断连」时无轮询兜底，终态前可能滞留旧快照；SSE 重连或终帧触发的重拉即自愈。
- 验收对齐（平台页陈旧期望）：`platform.js` 仍等待 brain 页旧文案「需求执行」（7015e99c 时代的 UI，现行工作台无此文案、HEAD 即红）；改为断言工具栏「开始新任务」+「能力库」页签，语义不放松（仍是大脑调度主页渲染主控件）。t12 brain 重开分支按钮名对齐实际文案「刷新运行」（原 `/^刷\s*新$/` 只匹配全部执行页的刷新按钮）。
- 备查（brain 子执行隔离工作区配置缺口，产品后续跟进）：brain 子执行在 `<execution_dir>/workspace` 隔离目录跑内嵌会话，`Config::load` 候选链只查工作目录本身 + `~/.opencoder/` 全局 + XDG、不向上遍历父目录，子会话解析不到节点工作区的 provider 配置 → 裸模型名落到 `openai/gpt-4o-mini` 默认，干净环境下缺 key 直接失败（步骤 ~400ms 失败、运行 `phase=failed`，表象为 t12 brain 段卡「等待事件」）。本任务在 t12/probe 里以写 `HOME/.opencoder/config.json`（fixture provider）作验收侧兜底；产品修复方向：子执行继承节点工作区配置或显式注入 provider。
- 备查（既有噪声，非本任务引入）：worker 终态后 outbox 回放仍会触发 authorize/publish 重放，日志刷 `action receipt changed` / `no pending plan publication`（成功与失败运行 alike）；不影响终态正确性，幂等修复另行跟进。
- 验证（验收三脚本全绿）：`t12_ui_verify.js` PASS（brain 段 `已通过` 断言不放松，含干净 HOME 环境复跑两次 probe 确认配置兜底生效）；`platform.js` PASS；`harness/codex.js` PASS（fixture 模式，含 Codex 托管参数/Modal 提交/断线重连段）。
- 验证：SPA 全量 vitest 88 文件 692 测试通过（新增 `src/fleet/detail/embeds.dom.test.jsx`：brain 嵌入不写 URL / todos 画布投影与 Inspector 联动 / 404 回退 / inline 门控双态；新增 `src/brain/workbench/tests/run.dom.test.jsx`；更新 `fleet.dom.test.jsx`、`team.dom.test.jsx`、`harness/launch.dom.test.jsx`、删除只测死代码的 `fleet/brain.dom.test.jsx`）；`scripts/build-spa.sh` 重建 dist 后 `check-spa-drift.sh` no drift；`scripts/acceptance/spa_responsive.js` 390×844 全部 11 页 0 溢出；无 Rust 变更。
