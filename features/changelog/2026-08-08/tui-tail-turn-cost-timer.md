Commit: (working-tree, pre-initial-commit)

# feat(tui): body 内容尾部计时器由 call-round 改为 whole-turn `[turn cost]`

## 背景

body 内容尾部的耗时计时原为 "call-round" 计时：只统计最近一段连续 Tool 调用的时长，
最后一个 Tool 一结束计时立即消失（`call_round_ms` 只覆盖 `elapsed_ms == None` 的 running
Tool，任一 finished Tool / 非 Tool block 都截断）。用户希望看到的是**整个 turn** 的耗时
（从 turn 开始到当前），并且在 turn 结束后**冻结停留**（不消失），方便回看刚结束的 turn
花了多久。

## 变更

- **`app_loop.rs` `tick_clock`**：由 4 参改为 5 参
  `(running, prev_running, last_clock, task_elapsed_ms, turn_elapsed_ms)`，新增 **turn
  clock**（`turn_elapsed_ms`，驱动 body 尾部 `[turn cost x]`）。语义：`false→true`（新
  turn 开始）snap `last_clock` 基线并**重置 turn = 0**（task 钟不重置）；`true` 态两钟共用
  同一 `dt` 累加；`true→false`（turn 结束）停止累加，task 保持当前值、turn 同样
  **冻结不归零**——turn 结束后尾部计时停留可见，直到下一 turn 开始才归零重计。
- **`app_display.rs` `display_tail_ms`**：改为 4 参 `(chat, subagent_focus, now,
  turn_ms)`。无 subagent 聚焦时直接透传 `turn_ms`（不再从 block 列表反推 call round）；
  聚焦运行中 subagent → `(now - started_at_ms).max(0)`；聚焦已完成 subagent → `0`。
  **删除 `call_round_ms`** 及对应 round 段反推逻辑。
- **`app.rs`**：run_app 状态新增 `turn_elapsed_ms`，每帧经 `tick_clock` 更新后传入
  `display_tail_ms`。
- **`render.rs` `render_body`**：尾部标签由 `[call {}]` 改为 `[turn cost {}]`；注释改为
  whole-turn 语义（turn 结束后冻结不消失）。

## 测试清单（crates/tui，全部为 unit）

| 行为 | 测试名 | 层 |
| --- | --- | --- |
| turn 开始（false→true）重置 turn 钟、不重置 task 钟 | `tick_clock_does_not_reset_task_on_turn_start` | unit(app_loop) |
| running tick 同时累加 task 与 turn 钟（同 dt 锁步增长） | `tick_clock_accumulates_task_while_running` | unit(app_loop) |
| turn 结束（true→false）冻结 turn 钟，idle 不推进，turn 2 开始重置 | `tick_clock_preserves_task_across_turn_end_and_idle` | unit(app_loop) |
| false→true 排除 idle 间隙，turn 2 起 turn 钟重新归零累加 | `tick_clock_false_to_true_excludes_idle_gap` | unit(app_loop) |
| 无聚焦时 `display_tail_ms` 透传 turn_ms | `no_focus_uses_turn_elapsed` | unit(app_display) |
| 聚焦运行中 subagent 显示 live elapsed（4 参签名） | `running_subagent_shows_live_elapsed` | unit(app_display) |
| 聚焦已完成 subagent 返回 0（4 参签名） | `done_subagent_returns_zero` | unit(app_display) |
| body 尾部显示 `[turn cost 42s]` | `body_shows_turn_cost_timer_at_content_tail` | unit(render) |
| turn_ms 为 0 时不渲染计时 | `body_hides_turn_cost_timer_when_zero` | unit(render) |
| 计时出现在内容文本之后同尾行 | `body_turn_cost_timer_after_content` | unit(render) |
| 内容行放不下计时换行到独立行（不丢弃） | `body_turn_cost_timer_wraps_to_own_line_when_full` | unit(render) |

## Gate

- 全量回归：`cargo test --workspace` → **2052 passed / 0 failed / 0 ignored**（当次实跑）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
- 行数：app.rs 800 / app_display.rs 251 / app_loop.rs 799 / render.rs 793（均 ≤ 800）
