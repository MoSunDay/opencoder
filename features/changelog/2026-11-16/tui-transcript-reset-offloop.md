# TranscriptReset 视图重建移出 UI 主循环（TUI）

一句话：clear-context（双击 Shift+Tab）触发的 `TranscriptReset` 视图重建不再在
UI select 循环里同步执行 —— 重建改为 `tokio::spawn` 后台任务，主循环在重建
期间继续渲染帧、响应键盘与确认倒计时；重建完成后一次性安装新视图，再把重建
期间到达的事件按序补折叠到新视图上。

## 背景与结论

旧行为：`fold_ui_events` 遇到本地 `TranscriptReset` 时同步调用
`replay_messages` 全量回放，期间 select 循环被占死，帧率、按键、
`/clear` 确认倒计时全部冻结；转录越长冻结越久（实测可达数秒）。

新行为：fold 遇本地 reset 立即返回（reset 必然是批次最后一个事件），把重建
交给后台任务；有界事件通道里排在 reset 之后的事件**原地保留**（不丢、不乱
序），主循环新增"重建完成"select 分支安装视图后继续折叠。远端（其他进程）
reset 仍走同步 `replay_messages`，语义不变。

与同日 [tui-compaction-no-inflight-echo.md](tui-compaction-no-inflight-echo.md)
互补：那份改动定义"重建视图回放什么"，本份改动定义"重建在哪个线程跑、
期间事件如何排队"。

## 变更清单

- `crates/tui/src/reset_rebuild.rs`（新增）
  - `ResetRebuildState { pending, inflight }`：`spawn` 用 `tokio::spawn` 起
    重建任务并以 oneshot 回传 `ChatView`；`wait_done` 取消安全（只在 oneshot
    解析后清空 pending 槽位）；`invalidate` 供会话切换丢弃在途结果。
  - 会话 id 不匹配 → 丢弃安装；oneshot 关闭（任务 panic）→ warn 并清槽，
    不死锁。
  - `#[cfg(test)] spawn_dying` 测试缝：任务早亡（sender 未发送即丢弃）
    产生与 panic 相同的关闭信号，配套单测锁定"None + 守卫复位"。
- `crates/tui/src/app_loop.rs`
  - `fold_ui_events` 新增 `reset` 参数；排空循环遇本地 `TranscriptReset`
    即停（reset 恒为批次末尾，含首事件即 reset 的情况）。
  - 非远端 reset 分支改为 `reset.spawn(...)` + 立即返回 `Proceed`；远端
    分支不变。
  - 抽出 `handle_ui_events`（fold + 空闲 `refresh_skill_mirrors` +
    `body_refresh_pending` + `dirty`）供主循环调用。
- `crates/tui/src/app.rs`
  - 初始化 `ResetRebuildState` 并取得 `reset_inflight: Arc<AtomicBool>`；
    evt 分支以该标志做前置守卫（避免与 wait_done 分支双重 `&mut` 借用）。
  - 新增 `reset_rebuild.wait_done(&session_id)` select 分支：安装视图并置
    `dirty/render_pending/body_refresh_pending`。
  - `switch_session` 成功后 `reset_rebuild.invalidate()`；
    `ArmClearConfirm`/`Cancel` 键路径置 `body_refresh_pending`；
    `handle_confirm_key`/`confirm_tick` 调用点传入 `&mut body_refresh_pending`。
- `crates/tui/src/app_loop_actions.rs`
  - `fire_clear_confirm` / `fire_clear_confirm_now` / `handle_confirm_key` /
    `confirm_tick` 全链路穿透 `body_refresh_pending`：排队 `[clear] 已排队`
    标记后、空闲 fire 的 `chat.begin_turn()` 后、Esc 取消分支均置位。
  - 空闲 fire 回显分支在回显非空时设置 `chat.pending_turn_echo`，修复 fire
    路径丢 User 轮次边界。
- `crates/tui/src/lib.rs` — 注册 `pub mod reset_rebuild;`。
- 测试（新增/更新）
  - `crates/tui/src/app_loop_tests/transcript_reset_offloop_tests.rs`（新增）：
    `DelayStore`（`get_session` 睡眠）模拟慢重建。
  - `transcript_reset_echo_tests.rs` 改为 spawn → `wait_done` 安装后再断言。
  - `act_clear.rs` / `act_clear_busy.rs` 调用点更新，并补
    `body_refresh_pending` 置位、`pending_turn_echo`、User 边界保持断言。
  - `render_gate_fuzz.rs` 新增纯 Gate 测试锁定"body 置位先于同轮渲染刷缓存"。
  - `fold_ui_events` 其余 27 处调用点（`app_loop_bugfix_tests.rs`、
    `app_loop_bugfix_tests/streaming_and_clock.rs`、`app_loop_tests/` 下
    cancel_keep_pending / compact_done_rekick / done_error_mirror_tests /
    plan_chip_consume_tests / stale_turn_done_tests / tok_cost_idle_refresh_tests）
    机械补 `&mut ResetRebuildState::default()` 参数。

## 功能 → 测试名 → 文件

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| fold 遇本地 reset 立即返回，重建期间帧/按键/倒计时继续走 | `fold_returns_and_ticks_flow_while_reset_rebuild_is_blocked` | `crates/tui/src/app_loop_tests/transcript_reset_offloop_tests.rs` |
| reset 后的事件按序留在有界通道，安装后才折叠到新视图边界下方 | `events_after_reset_stay_queued_until_install` | 同上 |
| 旧会话的迟到重建结果被丢弃 | `stale_rebuild_for_other_session_is_dropped` | 同上 |
| 安装后恢复在途回显 User 边界 | `transcript_reset_restores_in_flight_echo_below_rebuilt_view` | `crates/tui/src/app_loop_tests/transcript_reset_echo_tests.rs` |
| Done 清空 pending echo，后续裸 reset 不复活 | `done_clears_pending_echo_so_later_bare_reset_does_not_resurrect_it` | 同上 |
| 远端 reset 仍同步整视图回放，不走后台重建 | `remote_reset_still_replays_synchronously` | 同上 |
| 重建任务早亡（oneshot 关闭）返回 None 且守卫复位，不死锁 | `dying_rebuild_task_unblocks_instead_of_deadlocking` | `crates/tui/src/reset_rebuild.rs`（内置 `mod tests`） |
| 倒计时臂上/到点/取消/手动确认均置 body 刷新标记 | `slash_clear_context_arms_countdown_guard`、`expired_guard_queues_compound_when_running`、`esc_cancel_drops_countdown_chip`、`manual_confirm_while_running_interrupts_and_fires_now`、`shift_tab_repress_fires_armed_guard_now` | `crates/tui/src/app_loop_dispatch_cmd_tests/act_clear.rs` |
| 空闲 fire 提交规范 prompt 且保留轮次边界 | `fired_guard_submits_canonical_prompt_when_idle` | 同上 |
| subagent 排空中手动确认立即 fire | `manual_confirm_during_subagent_drain_fires_now` | `crates/tui/src/app_loop_dispatch_cmd_tests/act_clear_busy.rs` |
| body 置位先于同轮渲染刷新布局缓存 | `body_pending_flag_refreshes_cache_before_the_same_turn_render` | `crates/tui/src/app_loop_tests/render_gate_fuzz.rs` |
| 陈旧 TurnDone 丢弃 / 匹配 TurnDone 仍处理（回归） | `stale_turn_done_is_dropped_entirely`、`matching_turn_done_still_processes` | `crates/tui/src/app_loop_tests/stale_turn_done_tests.rs` |
| fire/reset 全链路折叠（集成回归） | `clear_context_folds_transcript_and_feeds_seed_to_model` 等 5 例 | `crates/tui/tests/act_clear_context_fold.rs` |
| 压缩重建回放口径不回归（集成回归，未改动用例） | `transcript_reset_drops_retained_in_flight_echo` 等 6 例 | `crates/tui/tests/compaction_summary_replay.rs` |

## 实测

定向测试（实际执行）：

```text
cargo test -p opencoder-tui --lib -- transcript_reset_offloop_tests transcript_reset_echo_tests render_gate_fuzz stale_turn_done_tests act_clear
# 21 passed / 0 failed（1812 filtered out；含 act_clear 11 例）
cargo test -p opencoder-tui --lib -- dying_rebuild_task remote_reset_still
# 2 passed / 0 failed（1831 filtered out；含 reset_rebuild 内置单测）
cargo test -p opencoder-tui --test act_clear_context_fold   # 5 passed / 0 failed
cargo test -p opencoder-tui --test compaction_summary_replay # 6 passed / 0 failed
```

- Clippy：`cargo clippy -p opencoder-tui --all-targets -- -D warnings` → 通过
  （exit 0，零警告），无代码级 `#[allow]`。
- `cargo check -p opencoder-tui --all-targets` → 通过（0 错误 / 0 警告）。
- `git diff --check` → 通过（无输出）。
- 行数上限：`app.rs` 800、`app_loop.rs` 799、`app_loop_actions.rs` 787、
  `transcript_reset_echo_tests.rs` 361（迭代文件 ≤800）；`reset_rebuild.rs` 186、
  `transcript_reset_offloop_tests.rs` 400（新增文件 ≤400）。
- **未运行全量测试**：仅运行上述 TranscriptReset 重建、clear 确认链路、
  渲染门控及相邻定向用例；`render_gate_fuzz` 等模块内其余用例未跑。
