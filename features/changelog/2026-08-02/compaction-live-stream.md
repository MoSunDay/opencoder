Commit: (working-tree, pre-initial-commit)

# 压缩摘要实时流式 + TUI 可折叠压缩块

## 背景
此前压缩（compaction）的摘要文本只在**完成后**以 `[context compacted] {截断摘要}`
单行 Marker 显示——用户在压缩进行时无任何反馈，完成后也看不到完整摘要（仅 100 字符）。
本次让压缩摘要**逐 delta 实时流式**投递，并在 TUI 中以可折叠块（与 Thinking 块同款）
渲染，默认折叠、点击表头展开。

## 变更

### 新增 `SessionEvent::CompactionDelta`
- **`crates/session/src/runner/event.rs`**：新增 `CompactionDelta(String)` variant；
  SSE 序列化 key = `compaction_delta`、payload `{"text": ...}`；
  `EventKind` 归入 `Compaction`（与终态 `Compaction` 同 kind，replay 时合并显示）；
  `from_sse_tests` roundtrip case 覆盖（unique kind 计数 18→19）。

### 压缩流改发 `CompactionDelta`
- **`crates/session/src/compaction.rs`**：`summarize` 把 summary chunk 从 `TextDelta`
  改为 `CompactionDelta`（避免污染助手回复流）；删除消费方 `select!` idle 守卫
  （idle 检测移入 `ChatClient` 中途重试）；新增 `Retrying` 分支（clear `text`，防跨尝试拼接）。

### TUI 可折叠压缩块
- **`crates/tui/src/compaction_block.rs`**（新增 87 行）：抽出 `render_collapsible`
  共享渲染 helper（Thinking / Compaction 复用同一渲染逻辑）；`ensure_compaction_open`
  + `CompactionHeader` 命中检测（点击表头折叠/展开）。
- **`crates/tui/src/chat.rs`**：新增 `ChatBlock::Compaction { text, collapsed }` variant；
  `apply` 流式追加 `CompactionDelta`；`Compaction` 终态事件改为推 `Compaction` 块（非 Marker）；
  `collapse_all_collapsible` 覆盖 Compaction；Thinking 渲染迁移到 `render_collapsible`。
- **`crates/tui/src/chat_types.rs`**：`ChatBlock::Compaction` variant + `CompactionHeader` struct。
- **`crates/tui/src/worker.rs`**：`CompactionDelta` 加入可丢弃 delta（UI 通道近满时安全丢弃）。
- **`crates/cli/src/display.rs`**：headless 模式忽略 `CompactionDelta`（no-op）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| CompactionDelta SSE 序列化 roundtrip | `from_sse_tests`（含 `CompactionDelta("cdelta")` case + kind 计数 19） | `crates/session/src/runner/event.rs` |
| Compaction 块折叠渲染（共享 render_collapsible） | `collapse_all_collapsible_collapses_every_thinking_block` | `crates/tui/src/chat_tests/thinking_state.rs` |

- 全量回归：`cargo test --workspace` → 1642 passed / 0 failed / 1 ignored
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
- 行数：`compaction_block.rs` 87（新增 ≤400）；`chat.rs` 818（迭代，较 799 增 19 行；
  已抽出 `compaction_block.rs` 共享渲染以缓解增长）

## Impact Surface
- TUI 用户：压缩进行时看到 `🗜 Compaction` 可折叠块实时增长；完成后可展开查看完整摘要。
- Web / SSE 消费方：新增 `compaction_delta` 事件类型（EventKind 仍为 `compaction`）。
- CLI headless：不受影响（CompactionDelta 静默忽略）。

## Related Docs
- [agents/session](../../agents/session/index.md)
- [既有压缩逻辑 changelog](../2026-07-06/plan-act-handoff-compact.md)
