Commit: (working-tree, pre-initial-commit)

# TUI 渲染节流：折叠 thinking 时跳过逐 delta 全量重绘

## 背景

reasoning 模型流式输出时，每个 SSE reasoning chunk → 一个 `ReasoningDelta` → 一次完整 `terminal.draw()`（ratatui 全缓冲 diff）。即使 thinking **已折叠**，header 里的 `(N lines)` 计数每 delta 都在变，所以折叠态没有任何短路。而且 `text.lines().count()` 每帧 O(n)，随 thinking 变长呈二次开销——CPU 越往后越高。

## 诊断 / 根因

`crates/tui/src/app.rs` 的主循环结构是**先 render 再 select**，且 `evt_rx.recv()` 每次只取一个事件。`render(...)` 在循环顶部**无条件全量重绘**，不论事件类型。折叠态 thinking 的 header 行数计数每 delta 变化，触发 ratatui 全缓冲 diff——逐 delta 渲染呈 O(n) × delta 数 = O(n²) 开销。

## 变更

### 1. `crates/tui/src/chat.rs`：新增 `last_thinking_collapsed()` 辅助方法
- `ChatView::last_thinking_collapsed(&self) -> bool`：当末尾块为折叠的 `Thinking` 块时返回 `true`，表示当前 reasoning 流被隐藏，可跳过逐 delta 重绘。

### 2. `crates/tui/src/app.rs`：门控渲染（`skip_next_render`）
- 循环前新增 `let mut skip_next_render = false;`。
- 循环顶部将 `render(...)?;` 包裹在 `if !skip_next_render { ... }` 中，之后重置 `skip_next_render = false;`。`let mut hits = MouseHits::default();` 保留在 guard 之外，确保 hit-testing 数据始终可用。
- `UiEvent::Session(sev)` 分支中，`chat.apply(&sev);` 之后追加：当事件为 `ReasoningDelta` 且 `chat.last_thinking_collapsed()` 时设 `skip_next_render = true`。

### 为什么安全

| 场景 | 行为 |
|------|------|
| 折叠态收到 `ReasoningDelta` | 模型更新（廉价 `push_str`），跳过 render |
| 300ms `anim_ticker` 触发 | 正常 render → spinner 动画 + header 行数刷新（节流到 ~3fps） |
| 用户点击展开 thinking | input 事件 → `toggle_thinking_at` → 下一帧 render 立即显示完整内容 |
| `TextDelta` 到来（thinking 结束） | `ensure_assistant_open` seal 掉 thinking，末尾块变为 `Assistant` → `last_thinking_collapsed()` 返回 false → render 恢复逐 delta |
| 其它事件（工具调用、Done、Error、输入） | 不受影响，正常 render |

## 涉及文件
- `crates/tui/src/chat.rs` — 新增 `last_thinking_collapsed()` 方法
- `crates/tui/src/app.rs` — `skip_next_render` 门控渲染 + ReasoningDelta 跳过逻辑
- `crates/tui/src/chat_tests.rs` — 4 个新单测

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 空 ChatView 的 `last_thinking_collapsed` 返回 false | `last_thinking_collapsed_empty_view` | `crates/tui/src/chat_tests.rs` |
| 折叠 thinking 块时返回 true | `last_thinking_collapsed_true_when_collapsed` | `crates/tui/src/chat_tests.rs` |
| 展开 thinking 块时返回 false | `last_thinking_collapsed_false_when_expanded` | `crates/tui/src/chat_tests.rs` |
| 末尾块非 Thinking 时返回 false | `last_thinking_collapsed_false_when_last_block_not_thinking` | `crates/tui/src/chat_tests.rs` |

- 全量回归：`cargo test --workspace` → 469 passed / 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：`chat.rs` 712 ≤ 800；`app.rs` 741 ≤ 800；`chat_tests.rs` 635 ≤ 800

## Impact Surface

- **TUI 用户**：reasoning 模型发起对话、thinking 折叠时 CPU 保持低位；展开后流式文字正常显示。渲染频率从逐 delta 降到 ~3fps（300ms anim_ticker），用户感知不变（spinner 动画 + 行数计数仍周期刷新）。
- **不影响** CLI / Web / session / store / llm —— 改动仅在 `crates/tui`。

## Related Docs
- [agents/tui](../../agents/tui/index.md)（已同步：render-skip 门控 + `last_thinking_collapsed`）
- [2026-07-12 tui-freeze-rootcause](./tui-freeze-rootcause.md)（同一日 TUI 性能/稳定性系列修复）
