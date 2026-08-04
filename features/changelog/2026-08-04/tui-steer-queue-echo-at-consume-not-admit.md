# fix(tui): queue/steer 回显从提交时移到执行时

## 背景

上一轮（`tui-compaction-streaming-queue-steer-admit-echo.md`）将 `queued:`/`steer:`
标记的推送时机从消费/提升时（`QueueConsumed`/`SteerConsumed`）前移到 admit（提交）
时。这导致**提交即回显**：用户按 Tab 排队或 Enter 插队后，正文立刻出现 `queued:`/
`steer:` 行，而该消息同时已出现在输入区上方的 pending 面板中——重复回显，冗余且
干扰阅读。更关键的是，真正执行时反而静默无声，用户无法感知消息已开始处理。

## 变更

反转 echo 时机：**提交时不回显，执行（consume/promote）时才回显**。

### A. 移除提交时回显

**`crates/tui/src/app.rs`** — 删除 `Steer`/`Queue` 分支中的 4 处
`push_marker`（steer 主分支 + 纯-skill 分支、queue 主分支 + 纯-skill 分支）
及其 `Echo immediately` 注释。提交时仅 `admit_input` + 维护 pending mirror
（`steer_items`/`queue_items`），不在 transcript 推任何标记。清理因此未使用的
`Modifier` import。

### B. 恢复执行时回显

**`crates/tui/src/chat.rs`** — `SteerConsumed { seq }` handler：从
`steer_items` 按 seq 查出 display 文本，push `steer:` marker，再 retain 删行。

**`crates/tui/src/app_loop.rs`** — `QueueConsumed { seq }` 处理块（位于
`fold_ui_events` 内，因 `queue_items` 是局部变量而非 `ChatView` 字段）：从
`queue_items` 按 seq 查出 display 文本，push `queued:` marker，再 retain 删行。
补 `Modifier` import。

## 测试覆盖

| 变更 | 测试 | 文件 | 层 |
|------|------|------|----|
| SteerConsumed 执行时回显 marker + 删行 | `steer_consumed_echoes_marker_and_drops_row` | `chat_tests/steer_echo.rs` | unit |
| SteerConsumed 未知 seq noop | `steer_consumed_unknown_seq_is_noop` | 同上 | unit |
| SteerConsumed 执行时回显（plan 场景） | `steer_consumed_echoes_marker_and_drops_entry` | `chat_tests/plan_card.rs` | unit |
| QueueConsumed 执行时回显 marker + 删行 | `fold_queue_consumed_echoes_marker_and_drops_entry` | `app_loop_tests/mod.rs` | integration |
| QueueConsumed 未知 seq noop | `fold_queue_consumed_unknown_seq_is_noop` | 同上 | integration |

- TUI lib：**872 passed / 0 failed**（当次实跑）
- workspace clippy：`-D warnings` 零警告（当次实跑）
- workspace build：零错误（当次实跑）

## Impact Surface

- **tui/app**: queue/steer 提交后不再在正文产生标记（仅 pending 面板可见）。
- **tui/chat + app_loop**: `steer:`/`queued:` 标记在消息实际被消费/提升时推入 transcript。
- 行为与用户预期一致：提交时无冗余回显，执行时有明确视觉反馈。

## Related Docs

- [agents/tui](../../../agents/tui/index.md) — queue/steer echo 语义修订（admit→consume）
