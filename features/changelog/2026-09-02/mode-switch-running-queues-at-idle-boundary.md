Commit: (working-tree)

# 模式切换 running 态入队：提交即入队、非 running 才生效（steer/queue 同族）

## 背景

语义澄清：Shift+Tab（act→plan）与 `/act_clear_context` 的正确模型是「提交与生效
分离」——**不是说 running 时不能提交，而是不能生效**。steer 与 queue 正是这一模
式的既有范本：running 时入队，非 running（idle 边界）才被 runner 自动提交并生效。

差距在 `dispatch_mode_switch`：running 时它直接拒绝（`[switch] busy — retry when
idle` marker）——这是「不能提交」，而 `/act_clear_context` 的 fire 路径
（`fire_clear_confirm`）running 时早已原文入队（idle 边界由 runner drain
intercept 应用）。两条同族路径行为不一致：同一个 running 语义下，倒计时守卫走
入队、纯切换走拒绝。

## 实现

- **`crates/tui/src/app_loop_actions.rs::dispatch_mode_switch`**：busy 分支从
  busy marker 拒绝改为 `handle_queue(mode.prompt())` + `push_history` ——与
  `fire_clear_confirm` 的 running arm 完全同形：raw 命令文本（`/act`/`/plan`）
  原文入队（queue 面板可见、↑ 可召回），runner 在 idle 边界 drain intercept
  应用（`control_cmd::apply` → `AgentSwitch` 事件 → TUI fold）。入队分支不动
  sys_tokens / mode flash——切换未落地，生效发生在 runner 消费行时；不设
  `running`/`begin_turn`（run 本就在飞）。
- **gate 收窄为父会话 running**：`gate_switch(*running)`（原
  `running || subagents_running > 0`）。live subagent 不算 busy——父会话 idle
  正是 steer/queue 被自动消费的时机，立即生效与 steer/queue 语义一致。
  `/compact` 的 `gate_compact(*running)` 原本就只看 running，模式切换至此对齐。
- **参数透传**：`dispatch_mode_switch` / `dispatch_slash_action` /
  `dispatch_command` 增加 `admit_tx/admit_st/queue_items/pending_images/
  history/hist_idx`（`session_id` 已有）——`app.rs`（Ctrl+T、Shift+Tab、`/`
  popup）与 `app_submit.rs`（自由文本）两条入口同源。
- **不变量**：文本命令路径（Enter=steer / Tab=queue）本就是入队语义未动；
  `fire_clear_confirm` running 入队未动；`key_handler` 层 `SwitchAgent` /
  `ArmClearConfirm` 产物未动；subagent 聚焦视图下文本模式命令仍按子会话无模
  式语义拒绝（防 steer 泄漏到子会话，与 running 生效语义无关）。

## 测试清单（功能点 → 测试）

| 功能点 | 测试 |
|---|---|
| running 时 `/act`、`/plan` 入队不生效（temp 行 + admit 请求 + 无 UiCmd + sys_tokens/flash 不动） | `app_loop_tests/switch_gate_tests.rs::mode_switch_while_running_queues_for_idle_boundary` |
| live subagent（父 idle）不算 busy：立即走 Run arm 提交 | `switch_gate_tests.rs::mode_switch_with_live_subagent_runs_at_parent_idle_boundary` |
| idle Run arm 保持既有契约 + 不入队 | `switch_gate_tests.rs::mode_switch_from_idle_submits_control_prompt` |
| popup 路径 running `/plan` 入队（dispatch 全链路） | `app_loop_dispatch_cmd_tests/mod.rs::slash_plan_while_running_queues_for_idle_boundary` |
| idle `/plan`、`/act` popup 提交保持 | `slash_plan_from_idle_submits_prompt`、`slash_act_from_idle_submits_prompt` |
| `/compact` running 仍拒绝且不入队（非 control command，无 drain intercept） | `app_loop_slash_action_tests.rs::slash_action_compact_running_pushes_busy_marker` |
| 入队 `/plan`、`/act`、`/act_clear_context` 的 runner 侧应用（idle 边界） | `crates/session/src/runner/drain_tests.rs`（`/plan`→ControlCmd、`/plan review`→Prompt、`/act` TUI 场景、`/act_clear_context review`） |
| fire 倒计时 running 入队（既有契约回归） | `app_loop_dispatch_cmd_tests/act_clear.rs::fired_guard_queues_compound_when_running` |

## 回归证据

- `cargo test -p opencoder-tui` → 1551 passed / 0 failed（lib，27+ 套件）+ 集成
  target 全绿
- `cargo test --workspace --no-fail-fast` → 247 target 全部 `test result: ok`，
  0 failed
- `cargo clippy --workspace --all-targets -- -D warnings` → 零告警
- `cargo build --workspace` → 零错误

## Related Docs

- [agents/tui](../../../agents/tui/index.md)
- [Shift+Tab 模式感知切换](../2026-09-01/act-shift-tab-mode-aware-switch.md)
- [模式切换 running gate（历史：拒绝语义）](../2026-08-08/mode-switch-running-gate.md)
- [clear-context 倒计时防护](../2026-08-29/clear-context-countdown-guard.md)
