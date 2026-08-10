# fix(tui): Compaction 标签与正文改为深紫色

## 背景

Compaction（上下文压缩摘要）块的标签与正文此前复用 `theme::local_color()`（`Color::Magenta`），
与 `/ps`、autopilot 等本地标记同色，视觉上偏亮、与 Thinking 粉色块区分度不足。希望 Compaction
的标签和文字都使用更深一档的紫色，让其作为次要信息在视觉层级上更收敛。

## 变更

- **`crates/tui/src/theme.rs`**：新增语义色槽 `compaction`——`pub const COMPACTION: Color =
  Color::Indexed(90)`（ANSI 256 色，RGB 135,0,135，比 Magenta 的 205,0,205 明显更深）、
  `Palette.compaction`（dark/light 两主题均 `Color::Indexed(90)`）、`pub fn compaction_color()`。
  不用 `Color::DarkMagenta` 的原因：ratatui 0.29 已移除该变体；用 `Indexed(90)` 与 palette 中
  既有的 `Color::Indexed(220)`（user 色）用法一致，且仍属 256 色兼容范围。
- **`crates/tui/src/chat.rs`**（`ChatView::flatten` Compaction 分支）：折叠 header
  （`📝 Compaction (N lines)`）与展开正文的样式由 `theme::local_color()` 改为
  `theme::compaction_color()`，标签与文字统一深紫色，仍无 BOLD。
- **`crates/tui/src/compaction_block.rs`**：doc 注释同步（不再称 "stays unstyled"）。
- **`crates/tui/src/chat_tests/compaction_state.rs`**：`compaction_header_and_text_are_uniform_purple`
  断言由 `theme::local_color()` 改为 `theme::compaction_color()`。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 折叠/展开的 Compaction 标签与正文统一深紫色、无 BOLD | `compaction_header_and_text_are_uniform_purple` | `crates/tui/src/chat_tests/compaction_state.rs` |

- 全量回归：`cargo test --workspace` → 通过
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：theme.rs 563 / chat.rs 713 / compaction_block.rs 122 / compaction_state.rs 164，均 ≤ 800

## Impact Surface

- 用户可见：TUI 内 Compaction 块（折叠标签行 + 展开摘要正文）颜色从 Magenta 变为更深紫色。
- 不影响：`local_color()` 仍服务 `/ps`、autopilot AP 徽标等本地标记；Thinking 粉色、bash 工具头
  Cyan、状态栏等其余颜色零变化；session/store/CLI 等边界不涉及。

## Related Docs

- [agents/tui](../../agents/tui/index.md)
- [tui-annotation-rename-and-pink-thinking](../../changelog/2026-08-09/tui-annotation-rename-and-pink-thinking.md)
