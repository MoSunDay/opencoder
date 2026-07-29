Commit: (working-tree, pre-initial-commit)

# feat(tui): 输入框 undo/redo + help 弹窗滚动/CJK 折行 + composer 模块拆分

## 背景

TUI composer 编辑体验与 help 弹窗存在多项短板：

1. **无撤销/重做** — 误删后只能从头输入。
2. **help 弹窗不可滚动、CJK 折行错位** — 快捷键列表超长时底部截断；窄终端下 CJK 字符按字节折行导致错位。
3. **composer.rs 单文件逼近行数上限** — 渲染、状态、cursor 数学混在一起，迭代困难。
4. **Tab-queue 误触** — 聚焦运行中 subagent 时按 Tab 仍会向父 session 入队一条 follow-up，产生非预期行为。

## 变更

### undo/redo（Ctrl+Z / Ctrl+Y）
- **`crates/tui/src/undo.rs`**（新增）：`UndoState` 快照栈纯函数。连续尾部 char 插入（增长 ≤ `COLLAPSE_THRESHOLD=3` char）折叠为一个 undo 步骤（"打一个词"）；Backspace / 删词 / 换行不折叠。提供 `init` / `snapshot` / `undo` / `redo` / `reset`。
- **`crates/tui/src/key_handler.rs`**：Ctrl+Z → `undo::undo`，Ctrl+Y → `undo::redo`；char / backspace / 换行 / 删词后 `snapshot`；submit / tab / skill / slash / agent 切换后 `reset` 清栈。
- **`crates/tui/src/app.rs`**：`undo_state` 在 `run_app` 初始化，按 `&mut` 传入 `handle_key`。

### help 弹窗滚动 + CJK 折行
- **`crates/tui/src/help.rs`**（新增）：`render_help(f, area, scroll)` 从 `render.rs` 的 `render_help_popup` 提取；`wrap_line` 复用 `composer::char_width` 做 display-width 感知折行（CJK 宽字符占 2 列）。
- **`crates/tui/src/key_handler.rs`**：help 打开时 Up/Down/PageUp/PageDown 调整 `help_scroll`，Esc 关闭。
- **`crates/tui/src/app.rs` / `frame.rs` / `render.rs`**：`help_scroll` 状态贯穿渲染链路。

### composer 模块拆分（行数控制）
- **`crates/tui/src/composer.rs`**：内联 `#[cfg(test)] mod tests` 移出，文件降为纯 cursor / 编辑数学函数。
- **`crates/tui/src/composer_tests.rs`**（新增，29 测试）：cursor_column / insert / backspace / wrap_rows / cursor_row_col / move_cursor_vertical / char_width / insert_str 上限守卫。
- **`crates/tui/src/composer_delete_tests.rs`**（新增，12 测试）：`delete_word_back` 全分支。
- **`crates/tui/src/lib.rs`**：声明 `pub mod {help, undo, welcome}`。

### 其他 UX 修复
- **`crates/tui/src/keybind.rs`**：`HELP` 文案英 → 中。
- **`crates/tui/src/key_handler.rs` / `app.rs`**：聚焦运行中 subagent 时 Tab → 新增 `KeyAction::QueueUnsupported`，app 显示瞬时提示且不触碰父 session。
- **`crates/tui/src/render.rs`**：`/model` 弹窗打开时隐藏光标（`model_menu.is_none()` 守卫）；多行 composer 续行补 prompt 宽度缩进（`ri > 0`）。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| undo 恢复文本 | `undo_restores_previous_text` | key_handler_tests.rs |
| undo 在 backspace 后 | `undo_after_backspace` | key_handler_tests.rs |
| help 滚动 ↓ | `help_open_down_arrow_increments_scroll` | key_handler_tests.rs |
| help PageDown 跳页 | `help_open_page_down_jumps_scroll` | key_handler_tests.rs |
| composer cursor / wrap / width | 29 项 | composer_tests.rs |
| delete_word_back | 12 项 | composer_delete_tests.rs |
| undo 纯函数 | 6 项 | undo.rs |
| help wrap / scroll | 6 项 | help.rs |

- 全量回归：`cargo test --workspace` → 1300 passed, 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：composer.rs < 800；help.rs 141 / undo.rs 124 / welcome.rs 42 / composer_tests.rs 342 / composer_delete_tests.rs 97（均 ≤ 400）

## Impact Surface
- TUI 用户：输入框支持 Ctrl+Z/Y 撤销重做；help 弹窗可滚动、CJK 正确折行；聚焦 subagent 时 Tab 不再误入队。
- 不影响：CLI / Web / session / store 边界。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
