Commit: 9f9f757519189f7a20f789b04c76dd29587da32c

# TUI 运行中禁止切换 act/plan——Ctrl+T 与 /act /plan 一律拒绝并提示

## 背景

- 此前 turn 运行中提交 act/plan 切换走「submit-always / apply-at-idle」：
  Ctrl+T 与 `/` 菜单经 `dispatch_mode_switch` 的 `SkipRunning` 分支把裸命令
  文本写入 queue（steer/queue 语义），Enter/Tab 分别 steer/queue，runner 在
  idle 边界排水时静默切换 agent。用户感知为「任务还在跑，模式却变了」。

## 变更

- `dispatch_mode_switch`（`crates/tui/src/app_loop_actions.rs`）：`SkipRunning`
  分支从入队改为直接拒绝——设置共享 busy flash「⚠ 任务运行中不可切换状态」，
  不再写 store 队列；签名瘦去 `admit_tx/admit_st/queue_items/pending_images/
  history/hist_idx/session_id`。`SwitchGate` 语义注释改为 BUSY = REFUSED。
- 覆盖入口（全部同一提示文案）：
  - Ctrl+T（`KeyAction::SwitchAgent` → dispatch gate）
  - Shift+Tab（act→plan，同上）
  - `/` 弹出菜单选中 /act、/plan（`dispatch_slash_action` Act/Plan arm）
  - Enter 提交裸 `/act` `/plan`：`key_handler.rs` 新增 `is_bare_mode_switch`
    判定（`split_control_prefix` 解析出 `SwitchAgent` 且无尾随任务文本），
    命中返回 `ModeSwitchBlocked`，输入行保留供 idle 重试
  - Tab 提交裸 `/act` `/plan`：同样 `ModeSwitchBlocked`，输入行保留
- 复合形式（`/plan review this`）仍是任务提交：Enter→Steer、Tab→Queue，
  模式切换随任务在 idle 边界生效，不受门禁影响（`is_bare_mode_switch` 不命中）。
- 子 agent 聚焦时所有 mode command 仍被阻止（既有行为，合并进同一分支）。
- `crates/web` 的 running mode 门禁与 runner 层 drain/steer 拦截保持不变
  （`tests/running_mode_switch_e2e.rs` 7 项契约照旧）。

## 测试

- `crates/tui/src/app_loop_tests/switch_gate_tests.rs`：
  `mode_switch_while_running_refuses_with_busy_flash`（act/plan 各断言 flash
  文案、无 `UiCmd`、running/sys_tokens 不动、无 transcript marker）。
- `crates/tui/src/app_loop_dispatch_cmd_tests/mod.rs`：
  `slash_plan_while_running_refuses_with_busy_flash`（原入队断言反转）。
- `crates/tui/src/key_handler_running_mode_tests.rs`：
  `running_enter_bare_mode_switch_blocked` / `running_tab_bare_mode_switch_blocked`
  （输入保留）、`running_enter_compound_mode_command_becomes_steer` /
  `running_tab_compound_mode_command_becomes_queue`（复合放行）。
- `crates/tui/src/app_loop_slash_action_tests.rs`：随 `dispatch_slash_action`
  瘦身清理 admit/queue 形参。
- 回归：`cargo test -p opencoder-tui --lib` 1720 通过；`clippy -p opencoder-tui`
  0 警告（`is_bare_mode_switch` 改用 `is_none_or`，e8bc16f4）。
- root e2e（fleet 二进制重建后）：operator 6 / todos 3 / team 1 / brain 3 /
  dag 6 / running_mode_switch 2 / tui_exit_restore 4 全过；
  `opencoder-session` 454、`opencoder-local` 全过。
