# TUI：任务运行中 `/model` 可切换会话模型（排队到当前 turn 结束生效，UI 不阻塞）

一句话：运行中不再拒绝 `/model`。纯模型选择在当前 turn 结束时由 worker 应用，UI 不同步等
ack；含 `providers` 的 patch 仍在运行中拒绝，`/config`、`/env` 维持 2026-11-27 的忙时拒绝契约。

## 背景与决策

- 2026-11-27 为避免 UI 事件循环同步等 worker ack 卡死，运行中一律拒绝 `/model`、`/config`、
  `/env`。用户希望任务运行中也能换模型。
- 本地 worker 严格串行：整轮 turn 在一个 `process_cmd(UiCmd::Prompt)` 内跑完，命令只在 turn
  之间被取出。因此运行中投递的命令天然在 turn 结束后生效——「排队到当前 turn 结束」不需要
  worker 新增任何调度逻辑，只需要一条**不带 ack** 的命令。
- 决策：
  - 新增 `UiCmd::ApplySessionDeferred { config, selection }`（无 ack 通道）。worker 处理时调用
    与 `ApplySession` 相同的 `apply_session(.., provider_changed = false)`，随后发
    `UiEvent::ModelCommit { selection, result }`。`ModelCommit` 与 `SessionEvent::ModelSwitch`
    走同一条 UI 事件通道，保序：UI 先看到转录同步 `chat.model`，再提交覆盖显示。
  - UI 侧 `queue_session_switch`：运行中仅接受纯模型选择（`selection_from_patch`），用
    `cmd_tx.try_send` 投递——UI 循环永不在命令通道上挂起；通道满/关闭时报告「未排队、运行时
    不变」。含 `providers` 的 patch 需要重建 client，属于空闲路径，运行中拒绝。
  - UI 侧 `commit_session_switch`：收到 `ModelCommit` 才提交 `chat.model_overrides` /
    `shared_model` / `model`，与空闲 `ApplySession` ack 路径共用 `commit_display`，两条路径最终
    显示状态一致。失败只推错误标记，显示保持旧模型。
  - 派发层：`SlashAction::Model` 无条件打开菜单；`gate_config_menu` 只覆盖 `/config`、`/env`。
    `handle_model_outcome(.., running)` 仍兜底拒绝 `Save` / `ReloadEnv`，仅放行
    `SaveSessionOnly`。
- 不改 worker 串行模型，不改 Web：`POST /sessions/:id/model` 在 drain 运行中仍返回 409，本次
  范围按「当前 turn」语义限定为 TUI。

## 标记文案

- 排队：`[/model] {mode} queued → {model} (applies when the current turn ends)`
- 提交：`[/model] {mode} switched (session only) → {model}`（与空闲路径一致）
- 失败：`[/model] session-only change failed: {error}; runtime unchanged`
- 未排队：`[/model] session-only change not queued: {error}; runtime unchanged`
- Provider 拒绝：`[/model] busy — Provider changes apply when idle; model selection is unchanged`

## 实现

- `crates/tui/src/worker.rs`：`UiCmd::ApplySessionDeferred`、`UiEvent::ModelCommit` 及
  `process_cmd` 分支；`gate_config_menu` 文档改为仅覆盖 `/config`、`/env`。
- `crates/tui/src/model_session_switch.rs`：抽出 `commit_display`；新增
  `queue_session_switch`、`commit_session_switch`。
- `crates/tui/src/app_loop_model.rs`：`running` 时 `SaveSessionOnly` 走 `queue_session_switch`，
  其余 worker-ack 结果仍 busy 拒绝。
- `crates/tui/src/app_loop_actions.rs`：`SlashAction::Model` 无条件打开菜单。
- `crates/tui/src/app_loop.rs`：`fold_ui_events` / `handle_ui_events` 新增 `config: &Config`
  参数与 `ModelCommit` 分支；keymap 三函数原样搬到 `app_loop_keymap.rs`（文件 800 行上限）。
- `crates/tui/src/app.rs`：`run` 改为 `pub use app_bootstrap::run`，调用处传入 `&config`。
- 测试侧 `fold_ui_events` 调用补 `&Config::default()`；三处 `UiEvent` 全匹配补 `ModelCommit`。
- `agents/frontend/tui/index.md` 同步运行中 `/model` 说明。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| worker deferred 成功：持久化覆盖、有效模型切换、client 不变，事件顺序 `ModelSwitch` → `ModelCommit Ok` | `deferred_session_command_reports_commit_after_model_switch` | `crates/tui/src/worker/tests_model_commands.rs` |
| worker deferred 重选同一模型：无 `ModelSwitch`，仍发 `ModelCommit Ok` | `deferred_session_command_without_effective_change_reports_commit_only` | `crates/tui/src/worker/tests_model_commands.rs` |
| worker deferred Store 失败：发 `ModelCommit Err`、无 `ModelSwitch`、worker 内存/client 不变 | `deferred_session_override_store_failure_reports_commit_error_without_worker_mutation` | `crates/tui/src/worker/tests_failures.rs` |
| 运行中 `Enter`+`n` 排队 `ApplySessionDeferred(Shared)`，外层 config / 显示 / client 不变，queued 标记；空闲后同按键发 `ApplySession` | `session_only_switch_while_running_is_queued_without_ack` | `crates/tui/src/app_loop_tests/config_busy_gate_tests.rs` |
| 运行中排队 → `ModelCommit` 折叠后提交显示（Shared 覆盖、`shared_model`、legacy 行转换下 Act 保持基值）并推 switched 标记 | `queued_selection_commits_display_only_after_model_commit` | `crates/tui/src/app_loop_tests/model_deferred_switch_tests.rs` |
| `ModelCommit` 失败：显示保持旧模型、失败标记 | `failed_commit_keeps_display_on_previous_model` | `crates/tui/src/app_loop_tests/model_deferred_switch_tests.rs` |
| Provider patch 运行中在排队前拒绝 | `provider_patch_while_running_is_refused_before_queueing` | `crates/tui/src/app_loop_tests/model_deferred_switch_tests.rs` |
| 无模型选择的 patch 不排队 | `selection_less_patch_is_refused_before_queueing` | `crates/tui/src/app_loop_tests/model_deferred_switch_tests.rs` |
| 命令通道满：`try_send` 失败报告未排队、不阻塞 | `full_command_channel_reports_not_queued_instead_of_blocking` | `crates/tui/src/app_loop_tests/model_deferred_switch_tests.rs` |
| `commit_session_switch` 无选择成功为静默 no-op；Act 选择的显示终态与空闲路径一致 | `commit_without_selection_is_silent`、`commit_display_matches_the_idle_switch_end_state` | `crates/tui/src/app_loop_tests/model_deferred_switch_tests.rs` |
| `/model` 运行中派发打开 `ModeSelect(Model)`、无 UiCmd、无 flash、turn 保持 | `slash_action_model_running_opens_menu_without_worker_command` | `crates/tui/src/app_loop_slash_action_tests.rs` |
| `/config`、`/env` 运行中仍拒绝（busy flash）；三个菜单空闲均可打开 | `slash_action_config_menus_running_refuse_with_busy_flash`、`slash_action_config_menus_idle_open_without_worker_command` | `crates/tui/src/app_loop_slash_action_tests.rs` |
| 既有链路回归：`/config` 保存与 Env 忙时拒绝、`model_session_switch` ack 路径、`fold_ui_events` 各 fold 测试、事件转发/重载 | 见下方命令 | `config_busy_gate_tests.rs`、`model_session_switch_tests.rs`、`app_loop_tests/*`、`app_loop_bugfix_tests*`、`worker/*` |

定向回归（2026-11-28）：`cargo test -p opencoder-tui --lib -- app_loop keymap app_helpers
app_tests model_outcome_tests env_model session_only switch_gate dispatch_cmd_tests worker` 356 通过
（含 `fold_ui_events` 各 fold 测试、事件转发 `delivery_tests`、`say_markdown_e2e`、`tests_reload`）；
`cargo clippy -p opencoder-tui --tests -- -D warnings` 零警告；`cargo fmt -p opencoder-tui -- --check`
无差异。未运行全量测试。

## 遗留说明

- Web `POST /sessions/:id/model` 在 drain 运行中仍 409（`model switch refused while drain running`），
  可后续对齐为同样的 turn 边界排队语义。
- 远端会话的 `/model` 仍在派发前被 `chat.remote` 规则拒绝，未改。
