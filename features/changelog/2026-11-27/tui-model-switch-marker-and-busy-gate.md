# TUI：去掉 `[model] <id>` 转录标记；任务运行中 `/model`、`/config`、`/env` 改为拒绝而非卡死

一句话：`SessionEvent::ModelSwitch` 不再往聊天转录插入 `[model] <id>` 行（模型切换已由
`[/model] … switched (session only)` 确认标记与标题/状态栏模型体现）；任务执行中打开或应用
`/model`、`/config`、`/env` 不再把 UI 事件循环挂起在 worker ack 上——与 `/act`、`/plan`、
`/compact` 同一契约：忙时拒绝并提示，空闲后再试。

## 背景与决策

- 用户反馈两条：(1) 聊天里切换模型后多出一行 `[model] xx`，属于重复提示；(2) 任务执行中在
  `/model` 切换模型会卡死。
- 卡死根因：本地 worker 严格串行——整轮 LLM turn 在一个 `process_cmd(UiCmd::Prompt)` 内
  跑完，`ApplySession` / `ReloadConfig` / `ReloadEnv` 只在 turn 之间被取出并 ack。UI 侧
  `switch_session` / `handle_model_outcome` 在唯一的事件循环里同步 `completion.recv().await`，
  期间 `evt_rx` 不再被消费：轻则整轮 turn 内界面冻结、Esc/Ctrl+C 无法处理；若剩余 turn 事件
  超过 `UI_EVENT_CAPACITY`（512，流式 `TextDelta` 很容易超过），事件转发器卡在
  `reserve().await`，worker 收尾的 `ui_forwarder.await` 不返回，`ApplySession` 永远不会被
  处理——形成真死锁。入口是空输入按 `/` 打开命令面板不受 `running` 限制，
  `SlashAction::Model | Env | Config` 又没有像 `Compact` 那样的 busy gate。
- 决策：沿用既有「需要 worker `&mut SessionState` 的命令忙时在 UI 拒绝、不排队不等待」的
  契约（`gate_switch` / `gate_compact` / `gate_clear_all`），新增 `gate_config_menu`；并在
  结果层（`handle_model_outcome`）与 Env drain 加一道兜底，覆盖「菜单已打开后 turn 才开始」
  的窗口（取消收尾、队列自动提交）。不改 worker 串行模型，不引入异步 ack 事件。
- `[model]` 标记：保留 `finalize_assistant()` 与 `chat.model` 同步（标题/状态栏、远端事件回放
  都依赖它），只去掉 `ChatBlock::Marker` 的推入；`ChatBlock::Marker` 变体与其他生产者不变。

## 行为与实现

- `crates/tui/src/chat.rs`：`ModelSwitch` 分支不再推入 `[model] …` 标记块。
- `crates/tui/src/worker.rs`：`ConfigMenuGate` + 纯函数 `gate_config_menu(busy)`，文档说明
  worker 只在 turn 间 ack、以及转发器有界队列导致的死锁链。
- `crates/tui/src/app_helpers.rs`：`config_menu_busy_flash(anim_tick)`——
  `⚠ 任务运行中不可切换模型/配置，请在任务结束后重试`。
- `crates/tui/src/app_loop_actions.rs`：`Model | Env(_) | Config` 合并为一个分支，Run 时调用
  `open_config_menu`，SkipRunning 时置 busy flash（不打开菜单、不发命令）。
- `crates/tui/src/app_loop_model.rs`：
  - `open_config_menu(action, …)`：原三处菜单构造原样搬入（`ModeSelect` for `/model`、
    `/config`，`EnvMenu::with_command` for `/env`）。
  - `handle_model_outcome(.., running)`：新增尾参；`worker_outcome_label(&ModelOutcome)` 给
    `Save`/`SaveSessionOnly`/`ReloadEnv` 标注所属命令（`/config`、`/model`、`/env`），忙时推入
    `[/model] busy — change not applied, retry when idle` 警告标记并返回，**不写盘、不发命令、
    不提交外层状态**；`Idle`/`Cancel`/`Quit` 不受影响（Ctrl+D 仍可退出）。
  - `drain_pending_env(.., running)`：忙时静默跳过，Env 命令保持 pending，空闲的第一次循环再
    执行。
  - Env 菜单带 pending 命令时，`handle_model_key` 会先执行该命令（含磁盘操作）再产生结果，所以
    `running` 时在调用 `handle_model_key` 之前就拒绝：命令保持 pending，不执行、不发命令，推
    `[/env] busy` 标记（`busy_marker(label)` 统一三个命令的忙时标记）。
- `crates/tui/src/app.rs`、`app_popups.rs`、`app_loop.rs`：传入 `running`、导出
  `worker_outcome_label`。
- 注释/文档同步：`worker/tests_reload.rs`、`tests/model_switch_persist.rs`、`theme.rs` 中关于
  `[model]` 标记的描述改为 `ModelSwitch` 事件 / `[/model] … switched` 标记；
  `agents/frontend/tui/index.md` 增补 busy gate 条目。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| ModelSwitch 同步 `chat.model`、收尾 assistant 且不加任何标记（完整 id 与裸 id） | `model_switch_updates_model_without_marker` | `crates/tui/src/chat_tests/agent_switch.rs` |
| `gate_config_menu` 空闲 Run / 忙时 SkipRunning | `gate_config_menu_runs_when_idle` / `gate_config_menu_rejects_when_running` | `crates/tui/src/worker/tests.rs` |
| `/model`、`/config`、`/env` 运行中派发：菜单不打开、无 UiCmd、busy flash 带 anim tick、无转录标记 | `slash_action_config_menus_running_refuse_with_busy_flash` | `crates/tui/src/app_loop_slash_action_tests.rs` |
| 三命令空闲时打开正确菜单且不发命令 | `slash_action_config_menus_idle_open_without_worker_command` | `crates/tui/src/app_loop_slash_action_tests.rs` |
| `worker_outcome_label` 只标注三个 worker ack 结果 | `worker_outcome_label_marks_only_worker_acked_outcomes` | `crates/tui/src/app_loop_tests/config_busy_gate_tests.rs` |
| 会话级切换忙时拒绝：无 `ApplySession`、外层 config/client/会话选择不变、busy 标记；空闲后同一按键正常发出 | `session_only_switch_while_running_is_refused_without_command` | `crates/tui/src/app_loop_tests/config_busy_gate_tests.rs` |
| `/config` 保存忙时在写盘前拒绝：磁盘模型不变、无 `ReloadConfig` | `shared_save_while_running_is_refused_before_disk_write` | `crates/tui/src/app_loop_tests/config_busy_gate_tests.rs` |
| Env drain 忙时保持 pending、不发命令、不推标记 | `drain_pending_env_while_running_keeps_command_pending` | `crates/tui/src/app_loop_tests/config_busy_gate_tests.rs` |
| Env pending 命令忙时在执行前被拒绝（未被消费、无命令、`[/env] busy`） | `env_pending_command_while_running_is_refused_before_execution` | `crates/tui/src/app_loop_tests/config_busy_gate_tests.rs` |
| 既有链路回归：`ReloadConfig` 同模型不发 ModelSwitch、ModelSwitch 事件持久化/resume、标题取 `chat.model`、`/compact` 忙时标记 | `reload_config_same_model_emits_no_model_switch`、`reload_config_success_swaps_model`、`model_switch_survives_resume`、`compute_display_title_prefers_live_session_model_over_config`、`slash_action_compact_running_pushes_busy_marker` | `crates/tui/src/worker/tests_reload.rs`、`crates/tui/tests/model_switch_persist.rs`、`crates/tui/src/app_loop_tests/display_title_tests.rs`、`crates/tui/src/app_loop_slash_action_tests.rs` |

定向回归（2026-11-27）：`cargo test -p opencoder-tui --lib -- agent_switch config_busy_gate
slash_action_tests model_outcome_tests env_model_tests env_model_failure_tests session_only_tests
gate_config_menu gate_compact gate_switch tests_reload display_title model_session_switch
dispatch_cmd_tests` 66 通过；`chat::tests::agent_switch chat::tests::tok_cost worker::tests::gate_
remote::` 50 通过；`cargo test -p opencoder-tui --test model_switch_persist` 2 通过；
`cargo clippy -p opencoder-tui --tests -- -D warnings` 零警告；`cargo fmt -p opencoder-tui -- --check`
无差异。未运行全量测试。

## 遗留说明

- Web SPA 仍把 `model_switched` 渲染为系统行 `model → <id>`（`reduce.js` / `modelModal.jsx`），
  本次只处理 TUI 转录，未改 SPA。
- 远端会话的 `/model`、`/config`、`/env` 仍按原规则在菜单打开前被拒绝（`chat.remote`）。
