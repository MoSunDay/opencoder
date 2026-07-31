Commit: (working-tree, pre-initial-commit)

# fix(session): subagent 30min 超时后正确标记 Cancelled 而非 Completed

## 背景

`task` 工具的两阶段 select（Phase 1 赛跑 / Phase 2 宽限 drain）中，
`TaskSignal::Timeout` 是唯一**不向子 agent 发送任何 cancel 信号**的分支：

- `HardCancel` — 父 cancel 已级联到子（`child_token()` 自动传播）
- `TurnCancel` — Phase 2 显式调用 `fire_child_turn_cancel` 触发独立 token
- `Timeout` — **什么都没做**

后果：超时后子 agent 在 15 s drain 窗口内继续盲跑。若碰巧在窗口内完成，
子 agent 的清理路径走正常完成分支，将 DB 任务标记为 **Completed** —
超时被静默吞掉。用户看到的是"30 分钟超时了，任务还是 running/completed"。

## 变更

### `crates/session/src/runner/execute.rs` — Timeout 信号传播 + 状态覆写

- **Phase 2 新增 cancel 传播**（:123-125）：当 `TaskSignal::Timeout` 胜出时，
  调用 `crate::fire_child_cancel(&child_cancels, &call_id)` 触发子 agent 的
  hard-cancel token，使其在 drain 窗口内及时停止，而非继续浪费算力。
- **`Ok(o)` drain 路径状态覆写**（:129-144）：子 agent 在 drain 窗口内完成时，
  其清理可能已将任务标记为 Completed 或 Failed（因无法感知超时）。当 signal
  为 Timeout 时，覆写 DB 状态为 Cancelled 并返回超时错误信息给父会话。

### `crates/session/src/lib.rs` — `fire_child_cancel` 单子精准取消

- 新增 `pub fn fire_child_cancel`（:57-76），按 `call_id` 从
  `child_cancels` map 中查找并取消单个子 agent 的 hard-cancel token。
  与 `fire_child_cancels`（广播全部）互补，与 `fire_child_turn_cancel`
  （turn-level token）对称。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 超时后任务标记 Cancelled（非 Completed） | `timeout_marks_subagent_cancelled` | `crates/session/tests/subagent_timeout_cancel.rs` |

- 全量回归：`cargo test --workspace` → 全绿（0 failures）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：`execute.rs` 554 ≤ 800；`lib.rs` 466 ≤ 800；`subagent_timeout_cancel.rs` 131 ≤ 400

## Impact Surface

- **用户可感知**：subagent 超时后 DB 任务正确显示 Cancelled，而非 Completed/
  Running。父会话收到清晰的超时错误信息。
- **不影响**：HardCancel / TurnCancel 路径（已有正确的 cancel 传播）；
  Store 抽象层；CLI/Web/TUI 前端。

## Related Docs

- [agents/session](../../agents/session/index.md)
- [既有相关 changelog：subagent hard-abort 不再遗留 Running](./fix-subagent-hard-abort-dangling-task.md)
