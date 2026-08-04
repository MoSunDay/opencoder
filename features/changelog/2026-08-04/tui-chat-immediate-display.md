Commit: (working-tree, pre-initial-commit)

# refactor: TUI chat 即时显示 — subagent 状态 / compaction 流式 / steer echo

## 背景

三个 TUI 渲染行为存在延迟或不一致：

1. **subagent 完成延迟**：多个 subagent 并发时，先完成的 subagent 状态/摘要被
   缓冲到 `pending_subagent_ends`，直到最后一个 sibling 完成才一起 flush。用户
   看不到即时反馈，失败 subagent 的摘要也延迟展示。
2. **compaction 不可见**：`CompactionDelta` 事件被忽略（空 match），用户在 LLM
   总结期间看不到任何进度；最终 `Compaction(summary)` 一次性渲染。
3. **steer marker 重复风险**：`SteerConsumed` 处理器内联嵌入 `steer:` 标记行，
   与 app.rs admit-time echo 存在重复渲染路径。

## 变更

### subagent 即时状态 — `chat.rs` / `chat_types.rs`

- 移除 `ChatView::pending_subagent_ends` 缓冲字段及 `flush_pending_subagent_ends()`
  方法。
- `SubagentEnd` 处理器直接调用 `mark_subagent_done()`，状态/摘要立即渲染。
- `hidden_assistant_idx` 清除逻辑保留（仅当 `subagents_running == 0` 时触发）。

### compaction 流式展示 — `chat.rs` / `chat_types.rs` / `compaction_block.rs`

- `ChatBlock::Compaction` 新增 `streaming: bool` 字段。
- `CompactionDelta(t)` → `open_compaction_streaming(t)`：增量追加文本、块展开。
- `Compaction(summary)` → `finalize_compaction(summary)`：写入完整文本、折叠。
- 渲染 match 从 `{ text, collapsed }` 改为 `{ text, collapsed, .. }`。

### steer echo 简化 — `chat.rs`

- `SteerConsumed` 处理器不再内联渲染 `steer:` 标记行（已移至 app.rs admit-time）。
- 仅保留 `steer_items.retain()` 移除已消费行。

### 测试更新 — `subagent_tests.rs` / `chat_tests/subagent.rs`

- `multiple_subagents_withhold_output_until_all_done`：断言改为第一个 sibling
  完成后摘要立即可见（旧：断言被缓冲不可见）。
- `failed_subagent_summary_buffers_then_flushes_with_sibling` →
  `failed_subagent_summary_shows_immediately_with_sibling`：失败摘要即时展示。
- 移除所有 `pending_subagent_ends.len()` / `.is_empty()` 断言。
- `done_while_subagents_running_reveals_preamble`：移除 flush 断言。

## 测试清单

| 闸门 | 结果 |
|------|------|
| `cargo test -p opencoder-tui` | 893 passed; 0 failed |
| `cargo test --workspace` | 1760 passed; 0 failed |
| `cargo clippy --workspace --all-targets -D warnings` | 0 warnings |
| `cargo check --workspace` | ok |

## 兼容性

- `ChatBlock::Compaction` 新增 `streaming` 字段 — 内部渲染类型，非公共 API。
- `pending_subagent_ends` 移除 — 内部缓冲字段，非公共 API。
- 行为变更：subagent 完成摘要从"批量延迟"变为"即时显示"，用户可见体验提升。
