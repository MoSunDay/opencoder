Commit: 0abfe0c77

# tui 模块

ratatui + crossterm 终端交互界面。细节以代码为准。

## 索引

- `src/app.rs`、`src/app_loop.rs` — App 状态与主事件循环
- [app_switch.rs](../../../crates/tui/src/app_switch.rs) — 会话切换意图原子化：
  `PendingSwitch` 把 `TaskPick` 与切换成功后采用的父会话绑在一起；显式切换
  （含 `/subtasks` Enter 下钻）先丢弃在途 `/agent` 查询，晚到的查询结果只在
  暂存槽为空时接受，失败切换不采用父指针
- [app_bootstrap.rs](../../../crates/tui/src/app_bootstrap.rs) — 启动、恢复运行态或
  远端绑定；进程级模型按 `--model` > `OPENCODER_MODEL` > 持久化会话选择 >
  per-mode config 解析，client 在显式覆盖后构造，resume 恢复持久化选择后再重应用
  同一显式胜者
- [app_task.rs](../../../crates/tui/src/app_task.rs) — 独立任务管理与本地新任务运行设置；
  `/task` New 继承当前 Shared 有效模型、完整 Shared/Act/Plan 会话元数据和完整
  `providers`（含 session-only 新增/编辑/删除），其他设置重新从磁盘加载；新 client
  基于同一个 `new_config` 构造，不沿用旧 client 的 provider 路由快照。继承先只写入
  新会话内存，首条输入持久化前不落 session 行，模型请求前不会出现空 task。
  `/task` Resume/Fork 先恢复持久化选择，再按 startup 相同优先级重应用显式胜者，
  并从覆盖后的 effective config 构造 client；只有恢复、client 构造、模型持久化及
  新 worker 准备全部成功后才同步外层 config/client/chat/context，任一失败保留原
  worker、模型状态、历史与草稿，不提交半切换状态。远端探测（`remote::load`）先于
  本地 resume 执行，其失败带 `task switch remote load for session <id>`；Resume 失败的
  错误链带 `task switch resume for session <id>`；切换失败处只按 session id + 错误链记
  warn（不记录标题 / 提示词 / 消息内容），用户可见提示不变；定向回归覆盖切换入口本地 / 远端加载失败的
  原子性、压缩会话的 OFFSET 尾部恢复与远端绑定任务的 Fork 拒绝（见
  [changelog](../../../features/changelog/2026-11-28/task-switch-remote-load-context.md)）
- [worker.rs](../../../crates/tui/src/worker.rs) — 分派 Server 或本地会话 actor，
  桥接 `SessionEvent` 到聊天渲染；turn 代标：`ResetCancel(CancellationToken, gen)`
  重置取消令牌并携带代数、`TurnDone(String, gen)` 回显同代，app 状态逐 turn
  `turn_gen` 比对，旧代残留的 cancel/act/drain 结果直接丢弃；回合失败以
  per-turn `Arc<AtomicBool>` 去重 Error 事件（runner 已发则不再补发兜底）
- `src/key_handler.rs`、`src/keymap.rs` — 键盘分发与映射（模式切换门禁）
- `src/composer.rs`、`src/chat.rs`、`src/render.rs` — 输入、消息渲染与渲染入口
- `src/chat_step_render.rs` — Step 阶梯渲染：阶梯纯用户控制、默认折叠，
  且只渲染已落定 step（`chat_steps.rs::settled_steps` 同门控渲染、
  行数镜像 `chat_headers.rs` 与点击目标 `chat.rs`
  toggle_tool_call_at/visible_targets 三处）；运行中
  （`progress_active`，Say 未开始）的 in-flight step 由 turn 实时区
  独占：`chat_flatten.rs::flatten_with` 把 live 区**内联**在运行中阶梯
  行之后、后续 Subagent 块之上（不再由 render.rs 拼在滚动窗尾部，
  `render_live.rs` 已删除），与转录共用同一滚动窗/滚动条（ViewportCache
  统一计行）；首个 step 流式期间阶梯**零行**（连 `N Steps` 行都没有），
  live 区是该 turn 唯一表面；Say 开始或 turn 结束（settle）后 live 区
  消失，完整 step 落入阶梯历史。live 区头行 `> streaming`
  （`LIVE_ROW_OPEN_PREFIX`）展开 / `▸ streaming`（复用
  GROUP_ROW_CLOSED_PREFIX）收起，dim warn 样式，默认展开：点击头行
  （`chat_headers.rs` LiveHeader + `render_hits.rs`
  LiveBtn/record_live_hits，`app_mouse.rs` 命中调
  `toggle_live_region`）或 Ctrl+L（`collapse_all_collapsible`）收起，
  每个新 turn（`begin_turn` / `reanchor_turn_after_user_echo`）重新
  展开；`chat_live_turn.rs::render_lines(steps, anim_tick, collapsed)`
  收起时仅头行；区内只有当前 step 的 thinking + 与阶梯同款的调用行
  （`push_step_details(…, show_results=false)`），**无 step 计数行、
  无 Say**。open Say 是转录内容：Say 一开始其上方阶梯整体退场（`chat_flatten.rs` 经 `chat_steps.rs::group_before_say` 检测下邻 open `Assistant`——中间隔 Subagent 块亦算——即跳过 `flatten_step_group`，行数/点击镜像 `chat_headers.rs` 同门控（同一查找），`open` 披露标记不动、seal 后恢复），流式期间独立形态（`❯ Say:` 头行（`chat.rs`
  ROLE_SAY_HEADER）+ running 提示 + raw body 全量，共享滚动/复制），seal 后才与邻接阶梯
  （同样可隔 Subagent 块，`chat_steps.rs::say_after_group`；`last_block_ends_blank`
  合并对自带收尾空行、不叠空行）合并为 `Say(n steps)` 头行（渲染正文 + 预览去重，步数随合并
  回归；见 [changelog](../../../features/changelog/2026-11-23/subagent-transparent-step-say-pairing.md)）；夹在阶梯与其
  Say 之间的 Subagent 段整体提到该 Say 对**上方**渲染（`chat_steps.rs::render_order`，
  flatten/headers/测试镜像共用；分组头、subagent 行、一个空行，再接 `Say(n steps)` 头行
  与正文——subagent 下一行是 Say 时补空行，收起态亦然，见
  [changelog](../../../features/changelog/2026-11-27/subagent-section-above-say.md)）；copy 模式经
  `flatten_for_copy`（`flatten_opts(.., include_live)`，配合
  `render_viewport.rs` build_for_copy/includes_live/refresh_copy_cache）
  排除 live 区，复制文本保持纯转录（与 SPA `stepsBlock.jsx` 同契约）
- `src/chat.rs` toggle_subagents / `chat_flatten.rs` subagent 分组头 — 每段极大连续的
  Subagent 块（非 Subagent 块分段）前置一行与 live 区同款样式的分组头（dim warn）：
  `> subagents`（`LIVE_ROW_OPEN_PREFIX` + `SUBAGENT_GROUP_LABEL`）展开态，其下每行
  `⤷ subagent [kind] prompt …` 保持完整明细（状态、时长、` [→ view]`、摘要），行本身
  不带标记；`▸ subagents`（`GROUP_ROW_CLOSED_PREFIX`）收起态只留分组头，subagent 行
  不渲染。状态是视图级 `ChatView::subagents_collapsed`（默认展开、跨 turn 持久、
  `begin_turn` 不重置）：`chat_headers.rs` 的 `subagent_group_headers()` 两种状态都登记
  分组头（`SubagentGroupBtn`），`subagent_headers()` 仅展开态登记行（`SubagentBtn`）；
  点击分组头（`app_mouse.rs`，经 `collapse_view` 作用于聚焦子视图）一次折叠 / 展开该
  视图全部分组，不进入子视图、不动滚动 / focus；点击可见的 subagent 行进入子代理视角；
  copy 模式保留分组头（转录结构，不同于瞬态 live 区）；Ctrl+L
  （`collapse_all_collapsible`）同样收起（见
  [changelog](../../../features/changelog/2026-11-27/subagent-fold-toggle.md)）
- [chat_steps_replay.rs](../../../crates/tui/src/chat_steps_replay.rs) — 回放侧
  Step 阶梯归一：`coalesce_steps` 折叠 Thinking、按 Say 闭合并组（与
  `chat_steps.rs` 共用阶梯语义）
- `session_ui/replay.rs` — 压缩摘要合成消息（标记来自 session
  `SUMMARY_PREFIX`）由回放入口拦截并就地渲染为 `Compaction` 折叠块，
  位于摘要消息的持久化位置（转录头部），`/task` 切换、resume 与
  mid-run 重建共用同一顺序（`summary + tail + 续跑`），与执行时
  `TranscriptReset` 重建所见一致；其余合成消息仍跳过；
  `compaction_block.rs` 的 `finalize_compaction` 对回放孪生块幂等去重。
  mid-run 压缩重建会丢弃在途 prompt 的 `User` 回显（保留尾部副本与
  `pending_turn_echo` 均不再重放），视图直接从 `Compaction` 摘要块继续；
  转录未记录该 prompt 的 clear-context 重建仍补回显边界；resume 回放不变。
  嵌套子视图（subagent）同契约：`ChatView::apply(TranscriptReset(msgs))` 调用
  `rebuild_view_from_messages`（`session_ui/reset_fold.rs`，`rebuild_after_reset` 的
  store-free 镜像）就地折叠，实时 `SubagentChild` 路由与 `reconstruct_child_view` 事件
  回放共用——子转录压缩后 `[Compaction, 保留尾段, 续跑]` 同序、一个摘要块恒在头部，
  连续压缩只留最新摘要；孙代理 `Subagent` 卡片随折叠重挂在派生消息之下（派生消息已
  折叠的按 orphan 追加），计数与可见卡片一致、运行中事件仍可路由
  （见 [changelog](../../../features/changelog/2026-11-27/subagent-compaction-fold.md)）
- [src/model_menu/](../../../crates/tui/src/model_menu/mod.rs) — `/model` 只选择
  Act/Plan，`/config` 选择 Shared/Act/Plan；Act/Plan 分别编辑
  `modes.act`/`modes.plan`，两行始终显示有效模型值（未单独覆盖时继承 Shared
  基值），生成参数表单的 `i`/Delete 以显式 `null` 恢复继承。
  Provider 新增、编辑、删除统一进入保存范围确认：`y`/`Y` 或 Enter 写磁盘，
  `n`/`N` 仅改会话，Esc 取消本次变更且不切换模型；菜单显示 `Active Env` 与
  解析后的 `Save target`。`/model` 没有 Shared 语义：菜单 scope、切换 patch
  （`modes.act|plan.model`）与会话覆盖写入都只涉及 Act/Plan；顶层 `model`（Shared
  默认）仅由 onboarding / `/config` 写入，`selection_from_patch` 不把它当作会话选择。
  会话层保留 legacy Shared 基值：首次 Act/Plan 选择把 Shared 与另一模式钉到 legacy
  基值（显示副本 `display_overrides` 镜像同一规则）；`{}` 或字段缺省表示继承配置，
  整体 `NULL` 保持 legacy 单模型语义。
  模型菜单与 `/cli`、`/skill` 显式使用 `EnvAware`：有活动 Env 时写入对应 Env 文件，
  无活动时回退 legacy `Auto`；`Auto` 和显式项目/全局路由不会被活动 Env 劫持
- [model_menu/env.rs](../../../crates/tui/src/model_menu/env.rs)、
  [env_view.rs](../../../crates/tui/src/model_menu/env_view.rs) — `/env`、`/envs`
  打开本地 Env 管理；支持 `/env [name]`、`use`/`activate`、`create`/`new`、
  `delete`/`remove`/`rm` 与 `off`/`none`/`deactivate`，菜单键为 Enter 使用、
  `n` 新建、`o` 不使用 Env、`d` 删除。切换后重载配置/client 与 CLI/Skill 状态、
  同步 MCP pool，并以 `{}` 清除 Shared/Act/Plan 会话覆盖；已落行会话持久化目标 Env
  的 Shared 基值，避免 resume 恢复旧 Shared；已消费的一次性 Skill body 标记保留
- [model_session_switch.rs](../../../crates/tui/src/model_session_switch.rs)、
  [worker/model_commands.rs](../../../crates/tui/src/worker/model_commands.rs) —
  会话级 Act/Plan 选择持久化到可选模式覆盖元数据，随 resume/fork 保留；
  完整配置 reload 才更新 legacy Shared 基值 `sessions.model`，不会覆盖独立 Shared
  会话选择。Provider patch 支持新增/编辑/`null` 删除，`api_key: null` 清空 key、headers
  整体替换，只合并进候选内存 `Config`，不写磁盘，也不属于可恢复的模型元数据。
  单独模型切换复用 client；Provider 实际变化才构造新 client。`ApplySession` 与
  `ReloadEnv` 均在 client 构造、会话持久化和 worker 运行时应用成功后返回 ack，UI
  仅在 `Ok` ack 后提交外层 config/client/chat/context 并显示成功；发送失败、worker
  结束或 Store/client 错误保留原 UI 运行时。普通 `ReloadConfig` 也按确认后提交：
  Shared 模型先持久化，worker 确认运行时应用后 UI 才提交外层 config/client；失败不发
  成功 `ModelSwitch`。`/cli`、`/skill` 投递失败不提交 live config，明确报告磁盘已保存
  但 reload 投递失败。`ModelSwitch` 事件只同步 `chat.model`、不插入 `[model] …` 转录行。
  worker 只在 turn 之间 ack 这些命令，UI 同步等待 ack 会冻结事件循环（转发器有界队列满后
  死锁），因此任务运行中 `/config`、`/env` 由 `gate_config_menu`（`worker.rs`）在派发层拒绝并
  显示 `config_menu_busy_flash`；菜单已打开后 turn 才开始时，`handle_model_outcome(.., running)`
  / `drain_pending_env(.., running)` 再兜底一次；Env 菜单的 pending 命令在 `handle_model_key`
  执行前即被拒绝、保持 pending（见
  [changelog](../../../features/changelog/2026-11-27/tui-model-switch-marker-and-busy-gate.md)）。
  运行中 `/model` 不受此门控：纯模型选择经 `queue_session_switch` 直接 push 到会话的
  `ModelSwitchQueue`（`crates/session/src/model_switch.rs`，不经过 worker 命令通道，UI 永不
  挂起），runner 在**当前这次 LLM 请求完成后**（`run_one_llm_call` 返回处、工具执行前）
  逐项应用：持久化覆盖 → 有变化发 `ModelSwitch` → 发 display-only 的
  `SessionEvent::ModelOverrideApplied { mode, model, error }`（`is_ephemeral_frame`，不落盘）。
  `fold_ui_events` 收到后通过 `commit_model_override` 提交 `model_overrides` /
  `shared_model` / `chat.model`，与 `ModelSwitch` 同批折叠，左上角模型即下一次请求使用的
  模型；失败的 pick 携带 error、显示保持旧模型、后续 pick 继续。含 `providers` 的 patch
  运行中仍拒绝（见
  [changelog](../../../features/changelog/2026-11-28/tui-model-switch-at-llm-boundary.md)）
- [app_display.rs](../../../crates/tui/src/app_display.rs) — `compute_display`
  展示标题优先取会话内当前模型（`chat.model`），空态才回退配置默认
- [agent_menu.rs](../../../crates/tui/src/agent_menu.rs)、
  [remote/selection.rs](../../../crates/tui/src/remote/selection.rs) — `/agent` 菜单
- [remote/](../../../crates/tui/src/remote/mod.rs) — Server HTTP/SSE、历史恢复与任务绑定
- [remote/transcript.rs](../../../crates/tui/src/remote/transcript.rs) — 远端消息恢复
- [remote/subtasks.rs](../../../crates/tui/src/remote/subtasks.rs) — `@capability` 派发；
  `cancel` 返回 `CancelOutcome`（响应中的终态 status，缺失/未知为 `None`，
  继续由观察器跟踪）
- [subtask_watcher.rs](../../../crates/tui/src/subtask_watcher.rs) — 一次性
  `@capability` 子任务后台观察：5 秒轮询 Server 至注册项终结（终态或
  会话类 Idle 收敛，经 `settle_status` 映射）后经 UI 通道各回报一次；
  失败按目标隔离：某目标自身连续 5 轮传输失败只把该目标按 error 结算，
  其余健康目标继续轮询。已由取消响应本地结算的目标从观察集移除（轮询
  前后均检查），不再回报。观察器独立于 session worker，
  `/task` 切换后写回仍流入父会话注册表
- [subtask_registry.rs](../../../crates/tui/src/subtask_registry.rs) — 纯函数
  注册表（`Vec<SubtaskRef>`）：`settle_status` 把 worker 只报 `Idle` 的
  agent/operator 成功收敛结算为 `done`，并把 `Interrupted` 在本地结算为
  终态 `interrupted`（Server 侧 `ExecutionStatus::terminal()` 不变，
  `is_terminal` 已包含该状态）；`settle_status_only` 只改状态、保留摘要与
  token（`cancelled` 清空摘要），供取消响应本地结算覆盖迟到的 `error`；
  `apply_update` 幂等应用回报并折算 token 增量；`bound_summary` 截断摘要
  至 2048 字符
- [subtask_detail.rs](../../../crates/tui/src/subtask_detail.rs) — `/subtasks`
  全屏视图：列表 + 详情（问题、服务端时间线、结果摘要与 token）；
  `j/k` 选择、`r` 刷新、`c` 取消（结果带回目标 id 与终态）、`Enter`
  进入子会话、`Ctrl+Enter` 把结果追加回主任务
- [app_subtask.rs](../../../crates/tui/src/app_subtask.rs) — `/subtasks` 键路由与
  tick 轮询：先收取消响应的终态、再收 watcher 回报；本地已结算目标丢弃迟到
  更新；追加回主任务前校验 `main_task_id`（无主任务时 flash 提示）
- [tests/subtask_restore_floor.rs](../../../crates/tui/tests/subtask_restore_floor.rs) —
  `/task` 切换恢复下限回归：切回恢复
  `max(回放用量, 快照 + 休眠期注册表增量)`，不回退也不重复计数
- [task.rs](../../../crates/tui/src/task.rs) — `/task` 列表；行内任务文本优先
  preview，缺失时回退 title；文本经终端安全清洗后按显示宽度判空，二者皆空
  （含仅控制字符/零宽字符）显示 ≥10 列占位；`src/app_submit.rs` 提交
- [control_helpers.rs](../../../crates/tui/src/control_helpers.rs) — 复合控制命令组装
- `src/notepad/` — 全屏文件树 + vim 编辑器
- `src/vim/` — vim 引擎
- `src/ts_mirror.rs` — tmux 会话冷启动恢复；`TsMirrorStore` 原样委托模式模型元数据的
  创建/读取/更新接口，保留 `NULL`、`{}` 与显式值，metadata-aware 创建仍触发既有
  ts 镜像 hook；`load_messages_after` 显式委托 inner 以保留原生 OFFSET 尾部读取
  （压缩头部不反序列化，避免被跳过的头部坏行拖垮 Resume）；registry 写者纪律——TUI
  从不长期持有 `ts.db` 写者：`maybe_wrap_at` 仅在 `ts.db` 已存在且能以只读
  `TsRegistry::open_reader` 打开时包裹（打不开则跳过镜像并继续），该 reader 常驻整个 TUI
  生命周期；每次镜像变更经 `TsRegistry::open` 取一个只用一次的写者、完成即释放——多写者 WAL
  下它与 ts 命令 / 其他 TUI 并存，事务层有界重试而不是被拒绝或推迟；
  `ts_mirror_tests.rs` 的 `mirror_mutations_commit_beside_a_live_writer`（见
  [changelog](../../../features/changelog/2026-11-28/database-multi-writer-wal-and-maintenance-lease.md)）
- [hooks.rs](../../../crates/tui/src/hooks.rs) — `~/.opencoder/hooks.json` TUI 事件命令
- [clipboard.rs](../../../crates/tui/src/clipboard.rs)、
  [app_loop_paste.rs](../../../crates/tui/src/app_loop_paste.rs) — 剪贴板图片转 PNG 附件
- [windows_console.rs](../../../crates/tui/src/windows_console.rs) — 控制台模式与 VT 输出
- `tests/` — 集成测试（agent_menu_catalog / question_flow / notepad_* 等）

## 边界

- 不持有 SessionState（worker 持有）；notepad 与本地 `!cmd` 不进模型 context。
- 本地任务复用共享 `SessionState`，Codex 执行与事件解码见
  [session](../../runtime/session/index.md)；远端任务使用 Server 注册的执行器。
- 远端绑定保存在 Harness 运行态；凭据仅由连接层读取 `OPENCODER_SERVER_TOKEN`。
- 切换意图（pick + 父指针）单点暂存与消费；取消响应的终态优先于观察器
  迟到的 `error`。

相关模块：[core](../../foundation/core/index.md)、[session](../../runtime/session/index.md)、
[control](../../platform/control/index.md)、[web](../web/index.md)。行为规则见
[Agent 调度平台](../../../features/platform/agent-platform/index.md)。
