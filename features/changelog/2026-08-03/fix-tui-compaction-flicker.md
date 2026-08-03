Commit: (working-tree, pre-initial-commit)

# fix(tui): CompactionDelta 改为 no-op，消除压缩块「出现→消失→重现」闪烁

## 背景

压缩（compaction）时 session runner 会先以 `CompactionDelta(String)` 逐块流式
推送摘要，最后发一个 `TranscriptReset`（清空 transcript），再发一个最终的
`Compaction(summary)` 事件。

TUI 此前在 `apply()` 里对 `CompactionDelta` 建块并累加文本。这导致渲染序列为：

1. `CompactionDelta` → 压缩块**出现**（流式文本）
2. `TranscriptReset` → 块被**销毁**（transcript 重置）
3. `Compaction(summary)` → 块**重现**（最终摘要）

表现为「出现 → 消失 → 重现」的可见闪烁。headless CLI（`display.rs`）早已忽略
delta、仅渲染最终 `Compaction`，TUI 与其行为不一致。

## 变更

### `crates/tui/src/chat.rs` — `apply()` 的 `CompactionDelta` 分支改为 no-op（核心）

```rust
// 之前：建块 + 累加文本
SessionEvent::CompactionDelta(t) => {
    self.ensure_compaction_open();
    if let Some(ChatBlock::Compaction { text, .. }) = self.blocks.last_mut() {
        text.push_str(t);
    }
}
// 之后：忽略
SessionEvent::CompactionDelta(_) => {}
```

效果：压缩块**只在**最终 `Compaction(summary)` 事件到达时渲染一次，消除闪烁。
`Compaction` 分支（finalize_assistant + push collapsed block）不变。

### `crates/tui/src/compaction_block.rs` — 删除死代码 `ensure_compaction_open()`

`CompactionDelta` 改为 no-op 后，`ensure_compaction_open()` 不再有调用方，删除。

### `crates/tui/src/chat_types.rs` — 更新 `ChatBlock::Compaction` 文档注释

反映新语义：「由最终 `Compaction(summary)` 事件创建一次；流式 `CompactionDelta`
被 TUI 忽略（摘要只渲染一次）。」

### 影响范围

- 仅 TUI 显示层。`worker.rs` 对 `CompactionDelta` 的分类 / 持久化逻辑不动。
- 无 trait / 数据形状 / 配置变更，无跨 crate 影响。
- headless CLI、web client 已忽略 delta，本次使 TUI 与其对齐。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| **`CompactionDelta` 是 no-op（不建块、不渲染文本；多次 delta 仍无累积；最终 Compaction 恰好建 1 块）** | `compaction_delta_is_ignored` | `chat_tests/compaction_state.rs` |
| Compaction 建折叠块 | `compaction_creates_collapsed_block` | `chat_tests/compaction_state.rs` |
| 展开/折叠切换 | `toggle_expands_and_collapses` | `chat_tests/compaction_state.rs` |
| collapse_all 覆盖压缩块 | `collapse_all_covers_compaction` | `chat_tests/compaction_state.rs` |
| header 显示行数 | `header_text_shows_line_count` | `chat_tests/compaction_state.rs` |
| header 点击切换折叠（适配） | `compaction_header_click_toggles_collapse` | `app_helpers_tests/mouse_tests.rs` |
| 渲染布局（适配） | render helper `compaction_view` | `render_tests/compaction.rs` |

> 原断言「两个 `CompactionDelta` 累积成一个块」的测试 `multiple_deltas_accumulate_in_one_block`
> 已删除：它验证的正是本次移除的流式建块行为。等价的 UI 覆盖（建块 / 切换 / 行数）
> 由上表保留的测试覆盖。新增 `compaction_delta_is_ignored` 锁定 no-op 语义，防止静默回退。

## 回归

- `cargo test --workspace` → **1684 passed / 0 failed**
- `cargo test -p opencoder-tui --lib` → **832 passed / 0 failed**
- `cargo clippy --workspace --all-targets -- -D warnings` → **0 warnings**
- `cargo build --workspace` → **PASS**
