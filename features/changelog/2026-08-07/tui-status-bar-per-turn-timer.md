# TUI 状态栏计时器：per-turn 计时 + 结束归零 + 尾部放置

## 背景

状态栏计时器 `run_elapsed_ms` 原先在整个 TUI 生命周期内单调累加、从不重置
（仅 `tick_clock` 中 `saturating_add`），实际显示"所有 turn 的总运行时间"。
用户要求改为 per-turn 计时：turn 进行中实时计时、turn 结束后归零清除、
计时器始终放在状态行尾部。

## 变更

### 1. `tick_clock` 增加 turn 边界双向重置

`crates/tui/src/app_loop.rs` 的 `tick_clock` 新增 `prev_running: &mut bool`
参数，在 `running` 状态转换边界重置计时：

- `false → true`（任务开始）：`run_elapsed_ms` 归零 + `last_clock` 快进到
  `now`（避免 idle 间隙计入）。
- `true → false`（任务结束）：`run_elapsed_ms` 归零，状态栏清除计时显示。
- `running` 保持 `true`（drain loop、subagent 执行期间）：正常累加，不重置。

### 2. 计时器 span 移至状态行尾部

`crates/tui/src/render.rs` 的 `render_status`：将 run-duration timer span
从 ctx 之后（spinner/status 之前）移到 spinner/status **之后**，成为状态行
最后一个元素（尾部）。仅在 `run_ms > 0` 时渲染（任务结束后归零自动隐藏）。

### 3. 调用方更新

`crates/tui/src/app.rs`：新增 `let mut prev_running = false;`，并传入
`tick_clock(running, &mut prev_running, &mut last_clock, &mut run_elapsed_ms)`。

### 4. 无需改动

- `chat.rs` `push_duration_span`：Tool/Subagent inline 计时器完全独立。
- subagent 执行期间 turn 计时自然继续（`running` 保持 `true`，无转换，不重置）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 任务开始归零 | `tick_clock_resets_elapsed_on_turn_start` | `crates/tui/src/app_loop_bugfix_tests.rs` |
| 同一任务内累加不重置 | `tick_clock_accumulates_without_reset_while_running` | `crates/tui/src/app_loop_bugfix_tests.rs` |
| 任务结束归零 | `tick_clock_resets_elapsed_on_turn_end` | `crates/tui/src/app_loop_bugfix_tests.rs` |
| 第二个任务再次归零 | `tick_clock_resets_again_on_second_turn_start` | `crates/tui/src/app_loop_bugfix_tests.rs` |
| 计时器位于状态行尾部 | `status_bar_timer_at_tail_after_status` | `crates/tui/src/render_tests/timer.rs` |

- 全量回归：`cargo test --workspace` → **2017 passed / 0 failed**（workspace 计数 ±8 非确定性：源于 session-crate 计时相关的集成测试，非本任务引入）
- TUI lib：`cargo test -p opencoder-tui --lib` → **1013 passed / 0 failed**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → Finished，零错误
