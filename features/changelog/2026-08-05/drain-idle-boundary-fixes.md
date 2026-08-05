# fix(session,tui): drain/idle 边界 turn_cancel 竞态修复 + 模块拆分

## 背景

Session runner 的 drain/idle 路径存在 4 个相互关联的竞态 bug，导致用户 steer/queue
在特定时序下被永久搁浅（stranded）：

1. **drain_mode 提前 Done**：drain_mode（web `drain_to_completion`）在队列耗尽后直接
   `Done + break`，但不检查是否有新 steer/queue 在处理期间到达，导致它们永远不被消费。
2. **has_pending_* 误杀**：`has_pending_steers`/`has_pending_queues` 在 turn_cancel
   fired 时提前返回 `false`，使 idle/drain 边界误认为「无待处理输入」而提前 Done。
3. **idle 缺 turn_cancel 复查**：idle 边界进入 `idle_drain` 前不复查 turn_cancel，
   一个 stale 的 cancel token 会阻止正常的 queue 消费。
4. **TUI Done 搁浅**：TUI `Done` 事件处理器不重同步 steer 镜像，也不在有搁浅输入时
   重启 drain_pending，导致 cancel/interrupt 后残留的 steer/queue 永久不可见且不消费。

## 变更

### `crates/session/src/runner/steer.rs`

- 新增 `cancel_guard(token)` 辅助：返回 hard-cancel 或 turn-cancel 的 future，用于
  biased `tokio::select!` 中提前退出。
- `claim_steers` / `claim_one_queued` / `has_pending_steers` / `has_pending_queues`
  改用 `tokio::select! { biased; hard-cancel => 早退; turn-cancel => 早退;
  正常逻辑 => 执行 }`。**关键修复**：`has_pending_*` 保留 hard-cancel 守卫但移除
  turn-cancel 守卫——turn_cancel 只表示「打断当前 LLM turn」，不表示「丢弃待处理输入」。
- 新增 `DrainOutcome` / `IdleAction` / `DrainModeAction` 枚举 + `drain_one_queued` /
  `idle_drain` / `drain_mode_step` 函数（从 mod.rs 提取，逻辑不变）。
- `drain_mode_step` 在队列耗尽后增加 `has_pending_steers || has_pending_queues`
  late-check：有则 `ConsumeNext`（继续 loop），无则 `Idle`。

### `crates/session/src/runner/mod.rs`

- `run_loop` 新增 `drain_mode: bool` 参数；在 turn 边界 `claim_steers` 之后增加
  drain_mode pre-consume 路径（调用 `drain_mode_step`）。
- idle 边界增加 `is_turn_cancelled` 复查：fired 则 reset 并 continue，不进入 `idle_drain`。
- 新增 `handoff_pending` 标志：`/act_clear_context` 保留 plan 时置 true，保持
  `drain_mode = false` 使 run_loop 执行 LLM turn 而非 idle。

### `crates/tui/src/app_loop.rs`

- `Done` 处理器：重同步 queue **和** steer 镜像（之前仅 queue）；若两者任一非空则
  `*drain_pending = true; *running = true`（重启 drain loop），否则正常 `*running = false`。
- `Error` 处理器：保持原逻辑（idle 不自动重启，避免错误循环）。

### 模块拆分（行数合规）

- `crates/session/src/runner/dedup.rs`（新建, 217 行）：提取
  `dedup_consecutive_bash_timeouts` + 8 个测试，使 `mod.rs` 从 828 → 590 行。
- `crates/tui/src/app_loop_paste.rs`（新建, 193 行）：提取 4 个 paste 相关函数
  (`paste_clipboard_image` / `paste_clipboard_image_silent` / `push_attach_marker` /
  `route_paste`)，通过 `pub(crate) use` 重导出保持调用方不变，使 `app_loop.rs`
  从 818 → 649 行。

## 核心不变式

- turn_cancel 仅打断当前 LLM turn，**不丢弃**待处理 steer/queue；idle/drain 边界
  必须消费完所有 pending 输入才能 Done。
- TUI `Done` 事件：若有搁浅输入则重启 drain_pending（`running = true`），否则 idle。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| turn_cancel 不阻止 has_pending_steers | `has_pending_steers_returns_true_when_turn_cancel_fired` | `session/src/runner/steer.rs` |
| turn_cancel 不阻止 has_pending_queues | `has_pending_queues_returns_true_when_turn_cancel_fired` | `session/src/runner/steer.rs` |
| turn_cancel 阻止 claim_steers（hard guard 保留） | `claim_steers_returns_empty_when_turn_cancel_pre_fired` | `session/src/runner/steer.rs` |
| turn_cancel 阻止 claim_one_queued（hard guard 保留） | `claim_one_queued_returns_none_when_turn_cancel_pre_fired` | `session/src/runner/steer.rs` |
| claim_steers 在 turn_cancel reset 后恢复正常 | `claim_steers_returns_data_after_turn_cancel_reset` | `session/src/runner/steer.rs` |
| TUI Done 有搁浅输入时 arm drain_pending | `done_with_pending_queue_arms_drain_pending` | `tui/src/app_loop_bugfix_tests.rs` |
| TUI Done 无搁浅输入时正常 idle | `done_with_empty_store_goes_idle` | `tui/src/app_loop_bugfix_tests.rs` |
| dedup 连续 bash timeout 提取后功能不变 (8 tests) | `dedup_tests::*` | `session/src/runner/dedup.rs` |

**全量回归**: `cargo test --workspace` → **1898 passed / 0 failed / 1 ignored**
（1 ignored 为 pre-existing）。
`cargo clippy --workspace --all-targets -- -D warnings` → 零警告。
`cargo build --workspace` → 零错误。
