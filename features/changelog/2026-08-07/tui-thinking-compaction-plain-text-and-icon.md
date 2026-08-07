Commit: (working-tree, pre-initial-commit)

# TUI Thinking/Compaction 去高亮 + Compaction 换图标 📦

## 背景
Thinking（💭）和 Compaction（🗜）内容块此前通过共享 `render_collapsible` 渲染，
统一施加 `theme::muted()` 前景色 + `ITALIC`/`BOLD` 修饰，形成视觉上的「高亮包裹」。
用户反馈该高亮不需要，要求去掉。同时 Compaction 的 🗜（压缩夹）在等宽终端字体里偏小、
渲染不一致，需替换为更清晰的图标。

## 变更

### 去掉 Thinking/Compaction 高亮样式
- **`crates/tui/src/compaction_block.rs`**：`render_collapsible` 改为输出纯文本（移除
  `theme::muted()` 前景色、`ITALIC`/`BOLD` 修饰）。删除不再使用的 `theme`/`Span`/
  `Style`/`Modifier` 导入；同步更新模块级与函数级 doc comment。折叠/展开的图标 +
  标签 + `(N lines)` 计数 + 2 空格缩进体全部保留。
- **`crates/tui/src/chat_types.rs`**：`Thinking` / `Compaction` 枚举变体 doc comment
  由 "dimmed/muted italic styling" 改为 "plain text, click-to-expand"。

### Compaction 换图标
- **`crates/tui/src/chat.rs:486`**：Compaction 渲染的 icon 由 `U+1F5DC`（🗜 压缩夹）
  改为 `U+1F4E6`（📦 包裹盒），语义贴合「压缩/打包」，终端渲染更清晰。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| Thinking 折叠显示行数计数 | thinking_header_shows_line_count_when_collapsed | crates/tui/src/chat_tests/thinking_state.rs |
| Thinking header 索引与 flatten 对齐 | thinking_headers_match_flatten_line_indices | crates/tui/src/chat_tests/thinking_state.rs |
| Thinking click 展开/折叠 | toggle_thinking_at_toggles_specific_block | crates/tui/src/chat_tests/thinking_state.rs |
| Compaction 折叠显示行数计数 | header_text_shows_line_count | crates/tui/src/chat_tests/compaction_state.rs |

- 全量回归：`cargo test --workspace` → 全绿（0 failed）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 编译干净
- 行数：compaction_block.rs 109 ≤ 800；chat.rs 751 ≤ 800；chat_types.rs 159 ≤ 800

## Impact Surface
- 仅影响 TUI 中 Thinking / Compaction 块的视觉呈现（去高亮 + 换图标）。
- 不影响：CLI headless / Web / session / store / 事件结构 / 折叠交互逻辑。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
