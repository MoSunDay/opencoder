Commit: (working-tree, pre-initial-commit)

# feat(session): queue 逐条 FIFO 排空，消费时回显 marker

## 背景

此前 `run_loop` 的 `queue_real_consumed` 闩锁限定每次 run 只消费一条 queued
真实 prompt，剩余行保持 pending 待下次显式提交。用户要求改为单次 run 内逐条
FIFO 排空直到队列空。同时 TUI 的 `queued:` marker 回显时机从提交时改为消费
时，使标记与实际处理顺序一致。

## 变更

### Session runner (`crates/session/src/runner/mod.rs`)

- 删除 `queue_real_consumed` 闩锁（声明 + idle 边界早停守卫 + 置位语句）。
- idle 边界行为：控制命令连续 `continue` 不消耗 LLM turn；真实 prompt break
  出内层让 LLM 跑一个 turn，该 turn 跑完回到 idle 边界继续取下一条 FIFO，
  直到 `claim_one_queued` 返回 None（队列空）才发 `Done`。
- doom-loop 守卫、tool-failure 守卫不受影响。

### TUI (`crates/tui/src/app.rs`, `crates/tui/src/app_loop.rs`)

- 删除 `push_queued_marker` 函数及其两处提交时调用（`app.rs` L567/L578）。
- `fold_ui_events` 的 `QueueConsumed { seq }` 块改为**消费时回显**：从
  `queue_items` 按 `seq` 查 `display`，`chat.push_marker("queued: {display}")`
  （粗体 warn 色 + 空行分隔），再 `retain` 删除该行。
- steer 提交时不回显、消费时批量 `steer:` 回显——不受影响。

## 测试

- `crates/session/tests/steer_followup.rs`：
  `queue_drains_all_fifo_in_single_run_then_done`（重写，2 条 queue 单次 run
  全排空后 Done）。
- `crates/session/tests/control_cmd.rs`：
  `queue_drains_control_cmds_between_real_prompts`（重写，[/plan, "do work",
  /act] 全部在同一 run 排空，最终 agent=act、队列空、QueueConsumed×3）。
- `crates/tui/src/app_loop_tests/mod.rs`：
  `fold_queue_consumed_pushes_marker_and_drops_entry`（重写，消费时 marker
  出现 + 行删除）。
- 全量回归 `cargo test --workspace`：1672 passed / 0 failed。
