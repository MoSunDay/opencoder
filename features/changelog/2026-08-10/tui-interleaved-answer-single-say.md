Commit: 0e0ec867c45170ffb244e38469baf7f4508bacc9

# TUI 交错推理正文归并与背压保真

## Context

Provider 在同一轮交错返回 `reasoning → text → reasoning → text` 时，TUI 以“最后一个块类型”判断是否新建 Assistant，导致一句回答被拆成多个 `❯ Say:`。同时 worker 会在 UI channel 接近满时丢弃 TextDelta，但既有 `TurnDone` 只做 markdown finalize，并未按注释从 store 补回正文，存在高压下回答缺字风险。

## Change Summary

- 抽出 `chat_stream` 纯状态转换：同一 LLM round 允许多个 Thinking、最多一个 Assistant；Thinking 统一在前，正文 chunk 不加字符直接拼入唯一 Say。
- `LlmRoundEnd` 封存本轮，Thinking/Assistant 依靠 `sealed`/`done` 各计一次 token；新 round、Tool、Marker、Subagent 等保持硬边界。
- replay 按 Thinking → 唯一 Assistant 重建，与 live 顺序一致。
- worker 为每条命令创建单一有序异步转发器；仅父级 TextDelta 可在 UI channel 接近满时降载，其余 child delta、reasoning、reset 与生命周期事件均按原序等待容量，不再经 `try_send` 静默丢失。
- 每次顶层 run 在同一有序队列中发送可靠 `AssistantFinal` 完成态；ChatView 仅在本轮 `turn_block_start` 后校准答案，覆盖部分丢 chunk 和全部丢 chunk 两种情况，不修改历史回答。

## Impact Surface

只改变 TUI transcript 的流式展示与背压恢复；SessionEvent、LLM 协议、Store schema、配置、环境变量和 headless 输出不变。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 交错 reasoning/text 只生成一个 Say 且正文精确拼接 | `interleaved_reasoning_keeps_one_losslessly_joined_assistant` | `chat_tests/thinking_state.rs` |
| 开放 Thinking 位于 Assistant 前时仍命中折叠重绘门控 | `interleaved_open_thinking_still_uses_collapsed_render_gate` | `chat_tests/thinking_state.rs` |
| round 边界与 context token 幂等计数 | `interleaved_round_finalization_counts_once_and_hard_bounds_next_round` | `chat_tests/thinking_state.rs` |
| 完成态修复部分丢失 chunk且不覆盖上一轮 | `completed_answer_repairs_dropped_chunks_without_touching_previous_turn` | `chat_tests/thinking_state.rs` |
| 全部正文 delta 丢失时补建唯一 Say | `completed_answer_creates_say_when_every_text_delta_was_dropped` | `chat_tests/thinking_state.rs` |
| 完成态只读取当前 run 新增消息 | `completed_assistant_text_is_scoped_to_messages_added_by_current_turn` | `worker/tests.rs` |
| Prompt 在 TurnDone 前可靠发送完整答案 | `prompt_sends_reliable_completed_answer_before_turn_done` | `worker/tests.rs` |
| 接近满载时仅顶层 TextDelta 可降载 | `ordered_forwarder_drops_only_repairable_parent_text` | `worker/tests.rs` |
| 完全饱和时可靠事件按序背压且不丢失 | `ordered_forwarder_backpressures_without_losing_reliable_events` | `worker/tests.rs` |
| replay 保持 Thinking-before-Say | `replayed_reasoning_restored_as_thinking_block` | `session_ui/replay_duration_tests.rs` |

## Gate

- 触及文件 `rustfmt --check`：通过。
- 全量回归：`cargo test --workspace` → **2308 passed / 0 failed / 0 ignored**。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告。
- build：`cargo build --workspace` → 成功。
- 行数：`chat.rs` 713、`chat_stream.rs` 192、`thinking_state.rs` 334，均满足门禁。

## Related Docs

- [TUI 模块](../../../agents/tui/index.md)
