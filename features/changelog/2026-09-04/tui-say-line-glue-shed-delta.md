Commit: 53519a1

# 中断→再提交后 Say 多行贴在一起：UI 通道 shed 的 TextDelta 行界保护 + 压缩后 AssistantFinal 修复

## Context

用户报告：中断任务后马上再提交，Say 多行内容换行丢失（行贴在一起）；再中断再提交又恢复——交替出现。

根因（机制单测实证）：`worker.rs::spawn_ui_event_forwarder` 在 UI 通道（容量 512）剩余容量 ≤ `DELTA_MIN_CAPACITY=64` 时丢弃 TextDelta（reasoning 洪泛 + UI 忙时的背压 shed）。被 shed 的块若携带 `'\n'`，两侧文本在 UI 拼接后粘成一行；**被中断的 run 永无 AssistantFinal** → 完成轮靠 reconcile 修复、中断轮修复缺席 → 粘行冻结在屏上。交替性 = 该轮通道压力是否触发 shed。

次要 bug：中途压缩（`TranscriptReset`）把 messages 换成更短列表后，`Prompt` 臂的 `message_floor` 越界 → `sess.messages.get(floor..)` 返回 None → 完成轮 AssistantFinal 静默丢失（又一个修复缺席点）。

## 变更

- **tui**（`worker.rs`）：
  - forwarder 维护 `shed_line_break`：shed 掉含 `'\n'` 的块后，给下一个**送达**的 TextDelta 前插一个 `'\n'`（其自身以 `\n` 开头则不叠加）。被 shed 的文本保持丢失（完成轮 reconcile 修复），但**行结构**在所有路径存活——中断轮亦然。
  - `LlmRoundEnd | TranscriptReset | Done | Error` 回合边界重置标志：封口当前 Say，分隔符不得泄漏成下一 Say 的前导空行。
  - `Prompt` 臂 `message_floor` 改 `AtomicUsize`，闭包监听 `TranscriptReset(msgs)` 更新为 `msgs.len()`：压缩后 AssistantFinal 切片不再越界。

## 测试

| 场景 | 用例 | 位置 |
|---|---|---|
| shed 含 `\n` 块后下一 delta 带分隔换行 | `forwarder_shedding_preserves_line_breaks` | crates/tui/src/worker/tests.rs |
| shed 块不含 `\n` 不插多余换行 | `forwarder_shed_without_newline_adds_no_separator` | 同上 |
| 分隔符不越过 LlmRoundEnd 泄漏 | `forwarder_shed_separator_does_not_leak_past_round_end` | 同上 |
| 下个 delta 以 `\n` 开头不双插 | `forwarder_shed_separator_not_doubled_when_next_starts_with_newline` | 同上 |
| 中途压缩后完成轮 AssistantFinal 仍送达 | `prompt_after_midrun_compaction_still_sends_completed_answer` | 同上 |
| 工具轮中断→再提交多行最终回答逐行分列 | `interrupt_tool_turn_then_resubmit_multiline` | crates/tui/src/chat_tests/interrupt_resubmit.rs |
| 随机中断/再提交/尾随 delta/reconcile 序列不粘行 | `fuzz_interrupt_resubmit_never_glues_lines` | crates/tui/src/chat_tests/interrupt_fuzz.rs |

测试时序说明：forwarder 测试用 current_thread 运行时 + `yield_now` 泵（`pump_forwarder`）确定性排序「先 shed、后腾容量」，容量用哨兵占位（+2 双哨兵避免交付阶段再次落回 shed 阈值）。回归：`cargo test -p opencoder-tui --lib` 1683 全绿。
