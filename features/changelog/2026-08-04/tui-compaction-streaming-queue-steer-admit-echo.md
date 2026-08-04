# feat(tui): compaction 流式回显 + queue/steer admit 即时回显

## 背景

两项 UX 缺陷：

1. **压缩摘要不可见**：`CompactionDelta`（summary LLM 流式 token）被 TUI
   完全忽略——用户在压缩期间看到的是冻结画面，只有最终 `Compaction(summary)`
   事件到达后才一次性渲染折叠块。CLI（`display.rs`）同样静默丢弃 delta。
2. **排队/插队项无即时反馈**：`queued:`/`steer:` 标记仅在 **消费/提升时**
   才推入 transcript（`QueueConsumed`→`app_loop.rs`、`SteerConsumed`→`chat.rs`）。
   用户按 Shift+Enter 排队或插队后，需等到 idle/turn 边界才能在正文看到回显，
   期间只有侧边队列面板可见——交互延迟感强。

## 变更

### A. Compaction 流式回显

**`crates/tui/src/chat_types.rs`** — `ChatBlock::Compaction` 新增 `streaming: bool`
字段。`streaming=true` 时块展开、`text` 随每个 delta 增长；`Compaction(summary)`
事件将其终结（覆写全文 + 折叠 + `streaming=false`）。

**`crates/tui/src/compaction_block.rs`** — 新增两个方法：
- `open_compaction_streaming(t)`：首个 delta 打开展开的流式块；后续 delta
  追加到同一块（镜像 `ensure_assistant_open`/`TextDelta` 语义）。
- `finalize_compaction(summary)`：若末块为流式块则覆写文本 + 折叠；否则创建
  新的折叠块（流式块已被 `TranscriptReset` 销毁的兜底路径）。

**`crates/tui/src/chat.rs`** — `CompactionDelta` handler 改调
`open_compaction_streaming`；`Compaction` handler 改调 `finalize_compaction`。

**`crates/tui/src/worker.rs`** — `is_droppable_delta` 移除 `CompactionDelta`
（含 `SubagentChild` 内层匹配）。delta 现驱动可见的流式块，丢弃会丢失数据。

**`crates/cli/src/display.rs`** — `CompactionDelta` 改为 dim 输出到 stderr
（`\x1b[2m…\x1b[0m`），与 TUI 一致地流式展示。

所有 `ChatBlock::Compaction` match 站点（`chat.rs`、`compaction_block.rs`、
`render`）均用 `..` 或显式绑定 `streaming`，编译零 non-exhaustive 警告。

### B. queue/steer admit 即时回显

**`crates/tui/src/app.rs`** — `KeyAction::Queue`/`KeyAction::Steer` 在
`store.admit_input` 成功后立即 `chat.push_marker` 推入 `queued:`（warn 色）/
`steer:`（accent 色）标记（均 BOLD）。纯-skill 提交（`$name` token）两条路径
同样即时回显。

**`crates/tui/src/app_loop.rs`** — `QueueConsumed` handler 移除 marker 推送，
仅 `queue_items.retain` 删行。

**`crates/tui/src/chat.rs`** — `SteerConsumed` handler 移除 marker 推送，
仅 `steer_items.retain` 删行。

**`crates/tui/src/steer_fire.rs`**（新增）— 从 `app.rs` 提取
`fire_steer_interrupt`，键盘 Enter 与鼠标 `>` 按钮共用，消除重复并控制
`app.rs` 行数（≤800）。

## 测试覆盖

| 变更 | 测试 | 文件 | 层 |
|------|------|------|----|
| delta 打开展开块 + 后续追加 | `compaction_delta_streams_into_expanded_block` | `chat_tests/compaction_state.rs` | unit |
| 无流式块时 finalize 创建折叠块 | `compaction_finalizes_without_streaming_block` | 同上 | unit |
| finalize 覆写流式块文本 + 折叠 | `compaction_creates_collapsed_block` | 同上 | unit |
| SteerConsumed 删行不推 marker | `steer_consumed_drops_row_without_marker` | `chat_tests/steer_echo.rs` | unit |
| SteerConsumed 未知 seq noop | `steer_consumed_unknown_seq_is_noop` | 同上 | unit |
| QueueConsumed 删行不推 marker | `fold_queue_consumed_drops_entry_without_marker` | `app_loop_tests/mod.rs` | integration |
| QueueConsumed 未知 seq noop | `fold_queue_consumed_unknown_seq_is_noop` | 同上 | integration |

- TUI lib：**870 passed / 0 failed**；integration **32 passed**（当次实跑）
- session crate：`cargo test -p opencoder-session` → **238 passed / 0 failed / 1 ignored**
- TUI clippy：`cargo clippy -p opencoder-tui --all-targets -D warnings` → 零警告
- workspace build：零警告；workspace test：1781 passed / 0 failed；workspace clippy 受范围外并发 `crates/web/src/api_ops.rs`（`result_large_err` lint）阻断，非本次代码

## Impact Surface
- **tui/chat**: compaction 块在 summary 生成期间可见（展开 + 实时增长）；`queued:`/`steer:` 标记在 admit 时即时出现。
- **tui/worker**: `CompactionDelta` 不再可安全丢弃。
- **cli**: 压缩 delta 流式输出到 stderr。
- **tui/app + app_loop + chat**: queue/steer marker 推送时机从消费/提升前移到 admit。

## Related Docs
- [agents/tui](../../../agents/tui/index.md) — 已同步修订 queue/steer echo 语义 + compaction 流式描述
