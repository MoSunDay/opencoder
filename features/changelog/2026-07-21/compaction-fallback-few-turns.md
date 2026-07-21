# 压缩回退：超阈值但轮数 ≤ tail_turns 时不再静默放弃压缩

## 背景

`should_compact()` 判定超阈值（预算 = `min(context_threshold, context_limit - reserved)`）后会触发 `compact()`。但 `compact()` 用 `split_index()` 求切分点，而 `split_index()` 在 **turn-start 数 ≤ `tail_turns`**（默认 2）时返回 `0`，于是 `compact()` 直接 `return Ok(None)`，只发一个瞬态 `Status("nothing to compact yet")` 事件。runner 见 `Ok(None) => {}` 后**原封不动把超大的整段 transcript 发给模型**——压缩从未真正发生。

因此任何 turn 数 ≤ 2 且已超预算的对话（例如一次 100k+ 的大段粘贴、或 1–2 轮无工具往返的长 Q&A）会突破阈值却不压缩，正是「超过阈值却没触发压缩」的现象。

## 设计要点

- **根因**：`split_index` 的「理想轮界切分」契约与「是否要压缩」是两件事；前者返回 0 被当成了「无需压缩」的信号，但 `should_compact` 已经判定要压缩。
- **修复**：新增 `compaction_split(messages, tail_turns) -> Option<usize>`，区别于 `split_index`（理想切分，过少 turn 时返回 0）。它在过预算场景下保证向前推进：
  - turn 数 > `tail_turns`：与 `split_index` 一致，保留最近 `tail_turns` 轮为 tail。
  - turn 数 ≤ `tail_turns` 但 ≥ 2：**回退**——压缩最旧一轮（`turn_starts[1]`），保留其余轮为 tail。
  - 仅 1 轮（`turn_starts=[0]`）且消息数 ≥ 2：保留最近一条消息、压缩其前内容（切分点 = 1）。
  - 仅当**空或单条消息**（真正无可压缩）才返回 `None`。
- `compact()` 改用 `compaction_split`：`let Some(split) = compaction_split(...) else { 真正的 no-op }`。其余 head/tail 切片、`previous_summary` 增量、`summarize` 调用、`summary_seq` 持久化算式对任意 split 均成立，未改动。
- `split_index` 行为**不变**（4 个既有 `split_index_*` 单测全绿），仅被测试直接调用，故加 `#[cfg_attr(not(test), allow(dead_code))]`。

## 变更

### `crates/session/src/compaction.rs`
- 抽出纯函数 `turn_start_indices(messages) -> Vec<usize>`（原 `split_index` 内联逻辑），`split_index` 改为委托它（行为不变）。
- 新增 `compaction_split(messages, tail_turns) -> Option<usize>`（带超预算回退）。
- `compact()` 入口：`split_index` + `if split == 0 { no-op }` → `let Some(split) = compaction_split(...) else { 真正 no-op }`。
- 新增 5 个单测：`compaction_split_fallback_summarizes_oldest_turn`、`compaction_split_fallback_two_tool_turns`、`compaction_split_single_turn_keeps_last_message`、`compaction_split_single_message_is_no_op`、`compaction_split_matches_ideal_when_enough_turns`。

### `crates/session/tests/compaction_and_model.rs`
- 新增端到端回归 `compaction_fires_when_over_budget_but_few_turns`：单条大消息（~5000 tokens > 1900 预算）+ run("go") → 恰好 2 个 turn-start（≤ tail_turns=2，即旧 bug 路径）；断言 transcript 收缩、首条为合成 summary、大消息被摘要掉、≥2 次 LLM 调用。已用「临时还原旧实现」验证：旧实现下该测试必失败（transcript 不收缩）。

## 已知限制

- 单条消息且无后续（真正只有 1 条）仍 no-op——无可压缩内容。
- 回退压缩一次仅摘要一轮；若压缩后仍超预算，下一轮 loop 会继续触发（渐进压缩），极端情况下 tail 内单条消息本身超预算时无法进一步压缩（保留最近上下文优先），由 doom-loop 守卫兜底。

## 验证（当次实跑，session scope）

| 命令 | 结果 |
| --- | --- |
| `cargo test -p opencoder-session --lib compaction` | 9 passed / 0 failed |
| `cargo test -p opencoder-session --test compaction_and_model` | 10 passed / 0 failed |
| `cargo build -p opencoder-session` | `Finished`，零告警 |
| 临时还原旧实现后跑 `compaction_fires_when_over_budget_but_few_turns` | FAILED（transcript was 20001, now 20012，证明测试有齿、旧实现确有 bug） |
| `cargo test -p opencoder-session`（全量） | 全绿（55 lib + 全部 integration） |
