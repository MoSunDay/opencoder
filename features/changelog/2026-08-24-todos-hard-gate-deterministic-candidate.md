# TODO 硬门禁完成后确定性收口

## 问题

schema v2 的 focused TODO 已按顺序完成所有 required tool calls 后，模型仍可能返回格式错误或受取消影响的 `blocked` Candidate。父工作流此前会重试整个 TODO，导致已经完成的 UI 副作用被重复执行，并可能最终命中外层执行超时。

## 修复

- schema v2 在当前 attempt 的最后一个 required tool result 成功后立即取消 focused session 的后续模型轮次。
- 从当前 attempt 的持久化 ToolUse/ToolResult 重新计算严格有序、一对一工具门禁。
- 仅当 required tool calls 非空且门禁完整通过时，运行时可从工具证据生成确定性 Candidate。
- schema v1、失败或缺失的工具门禁、以及未触发运行时完成门禁的合法 `blocked` Candidate 保持原行为。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| malformed Candidate 的门禁完成兜底 | `schema_v2_completed_gate_recovers_malformed_candidate_without_reexecuting_tools` | `crates/todos/src/execution.rs` |
| 兜底边界 | `candidate_recovery_requires_schema_v2_nonempty_completed_gate` | `crates/todos/src/execution.rs` |
| 完成门禁立即停止 | `completion_gate_stops_only_after_all_required_calls_succeed_in_order` | `crates/todos/src/execution.rs` |
| 失败/乱序不停止 | `completion_gate_does_not_stop_on_failed_or_out_of_order_calls` | `crates/todos/src/execution.rs` |
| 运行时完成覆盖取消尾帧 | `runtime_completed_gate_overrides_cancelled_blocked_tail` | `crates/todos/src/execution.rs` |
| 非运行时完成保留 blocked | `blocked_candidate_is_preserved_when_runtime_did_not_finish_the_gate` | `crates/todos/src/execution.rs` |
| 真 Store + Mock LLM 单次工具调用收口 | `schema_v2_stops_after_last_required_tool_without_second_action_turn` | `crates/todos/tests/hard_gate_completion.rs` |

- 定向回归：`cargo test -p opencoder-todos` → 74 passed / 0 failed。
- Clippy：`cargo clippy -p opencoder-todos --all-targets -- -D warnings` → 零警告。
