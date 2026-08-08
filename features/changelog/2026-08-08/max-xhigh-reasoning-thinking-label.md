Commit: (working-tree, pre-initial-commit)

# feat(llm,tui): max/xhigh 推理可靠显示 `💭 Thinking` 标签

## 背景

`max`/`xhigh`（以及部分 `high`）推理档位下，TUI 常常**看不到** `💭 Thinking` 标签。
根因不在配置层（`reasoning_effort` 一直原样透传，`Reasoning` 枚举早就支持
Max/XHigh），而在**上游解析**：高档位推理时很多模型的 SSE 线格式会变，推理内容不再
以 `delta.reasoning_content` 字符串下发，而改用 `reasoning`/`thinking` 等别名、
`delta.content` 结构化数组块、或仅在末帧 `choice.message.reasoning_content` 一次性给全。
只要字段名对不上 `reasoning_content`，就产生 **0 个** `ReasoningDelta` → 标签不出现。

## 变更

### 主修复（根因，`crates/llm/src/client.rs`）
- 新增 `extract_reasoning(obj)`：按优先级接受多个推理字段别名
  `reasoning_content`/`reasoning`/`thinking`/`reasoning_summary`/`chain_of_thought`/
  `analysis`/`thoughts`，支持字符串或字符串数组（数组内拼接）。
- 支持 `delta.content` 为**结构化数组**（`[{type:"text"|"thinking"|"reasoning",...}]`）
  时按序发射 text 与 thinking/reasoning 块 —— 高档位常见格式；文本取 `text` 或
  `content`。
- `handle_event` 末帧回退：`finish_reason`/末帧时扫描 `choice.message.*`（含别名/结构化
  content 数组），补上「一次性整段下发」的情况，发射单个 `ReasoningDelta`。
- **双发射守卫（评审 gap #1 加固）**：`emit_delta` 返回本帧是否发射过推理，`handle_event`
  用跨帧 `streamed_reasoning` 布尔——只要本 turn 已通过 delta 流式下发过推理，`choice.message`
  回退即跳过。修复「delta 流式 + 末帧 message 全量」双通道 provider 的推理文本双发射：
  UI Thinking 块重复、且工具轮次下 `reasoning_buf` 会重复并持久化 `ContentBlock::Reasoning`
  回传给 API。单通道 message-only provider 不受影响（无 delta 推理时回退照常发射）。
- 结构化 content 数组存在时跳过别名回退，避免同帧重复发射同一 thinking 块。
- 结构化数组按**原始流序**迭代，text 与 thinking 保持顺序。

### 次要安全网
- **`crates/tui/src/worker.rs`**：将 `ReasoningDelta`（及包裹它的 `SubagentChild`）从
  `is_droppable_delta` 可丢弃集合中移除。原因：`ReasoningDelta` 是 `💭 Thinking` 标签的
  唯一触发器，且非工具轮次的 reasoning 不持久化、`TurnDone → finalize_assistant()` 只从
  store 重建 text —— 丢弃即永久丢失思考块。TextDelta 仍可丢弃（text 总会重建）。
- **`crates/tui/src/session_ui/replay.rs`**：`replay_one` 在 Assistant 分支还原
  `ContentBlock::Reasoning` → `ChatBlock::Thinking`（collapsed + sealed），使
  resume/compaction 后历史里的 `💭 Thinking` 标签不再消失。

## 测试清单

| 行为 | 测试名 | 层 |
| --- | --- | --- |
| 标准 `reasoning_content` 字符串产出推理 | `extract_reasoning_reads_reasoning_content_string` | unit(llm) |
| 别名字段（reasoning/thinking/summary/chain_of_thought）产出推理 | `extract_reasoning_reads_alias_keys` | unit(llm) |
| 字符串数组拼接 | `extract_reasoning_joins_string_array` | unit(llm) |
| 结构化 thinking/reasoning 块提取 | `extract_reasoning_reads_structured_thinking_blocks` | unit(llm) |
| 显式键优先于结构化块（防重复） | `extract_reasoning_prefers_explicit_key_over_structured` | unit(llm) |
| 纯 text content 不误判为推理 | `extract_reasoning_ignores_plain_text_content` | unit(llm) |
| 无任何推理字段时返回 None | `extract_reasoning_returns_none_when_absent` | unit(llm) |
| 空别名跳过、取下一非空 | `extract_reasoning_skips_empty_alias` | unit(llm) |
| emit_delta 别名字段发射 ReasoningDelta | `emit_delta_emits_reasoning_for_alias_field` | unit(llm) |
| emit_delta 结构化块按序发射 thinking+text | `emit_delta_emits_reasoning_for_structured_thinking` | unit(llm) |
| 末帧 message.reasoning_content 回退发射 | `handle_event_emits_reasoning_from_message_fallback` | unit(llm) |
| 无 reasoning 时不发回退帧 | `handle_event_skips_message_fallback_when_no_reasoning` | unit(llm) |
| request_body xhigh/max/all 档位原样透传 | `body_includes_reasoning_effort_xhigh`/`_max`/`body_passes_all_effort_levels_verbatim` | integration(llm) |
| 通道饱和时 ReasoningDelta 绝不丢弃 | `forward_event_never_drops_reasoning_delta` | unit(tui) |
| replay 还原 Reasoning → Thinking 块 | `replayed_reasoning_restored_as_thinking_block` | unit(tui) |
| delta 已流式 + message 同时存在不双发 | `message_fallback_does_not_double_emit_after_streamed_reasoning` | unit(llm) |
| 无 delta 推理时 message 回退仍发射 | `message_fallback_fires_when_no_delta_reasoning_streamed` | unit(llm) |

## 回归

- `cargo test -p opencoder-llm` → 66 lib（含新增 2 项守卫测试）+ connect_retry 2 / headers 6 / lower_messages 15 / mock_contract 4 / request_body 9 / stream_retry 4 / stream_timeout 2 / doc 0 passed，0 failed
- `cargo test -p opencoder-tui --lib` → 1029 passed，0 failed
