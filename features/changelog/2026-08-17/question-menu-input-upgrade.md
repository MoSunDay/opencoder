Commit: (working-tree, post-a315873)

# question 弹窗：选中高亮与焦点解耦 + Input 终端快捷键与自适应高度

## Context

question 弹窗（plan 模式 `question` 工具交互）有两处体验短板：① 选中项的高亮（`▸`/accent/BOLD）此前绑定 `focus == Options`，Tab 切进 Input 追加信息后高亮整体消失——但 `selected` 状态仍在，用户无法回看当前确认目标；② Input 框固定 3 行、单行横向滚动（`input_window` 窗口），只支持 Left/Right/Backspace，无 readline 快捷键、无软换行、无多行光标移动，与主输入框能力差距大。

## Change Summary

- **高亮解耦**（`view.rs::content_lines`）：选项行高亮条件从 `focus == Options && selected == index` 收敛为 `selected == index`（含 Custom 行）；焦点归属由 Input 框边框色（warn=聚焦 / muted=失焦）单独表达。
- **Input 快捷键**（`state.rs::handle_custom_focus`，新增 `custom_ctrl_key`/`custom_alt_key`）：本地 `insert_at_cursor`/`backspace_at` 删除，全面复用 composer 纯函数；新增 Ctrl+A/Home 行首、Ctrl+E/End 行尾（按逻辑行，非整缓冲）、Ctrl+U 清空、Ctrl+K 删到末尾、Ctrl+W/Alt+Backspace 删前词、Alt+B/Alt+F 词移动、Delete 前向删除（新 `delete_forward`）、Shift+Enter/Alt+Enter/Ctrl+J 插入换行（Enter 仍确认）；未匹配的 Ctrl/Alt 组合吞掉（tmux Esc-merge 防垃圾字符，同主输入框）。
- **Up/Down 多行语义**：光标不在首视觉行时经 `composer::move_cursor_vertical` 行内移动（跟随软换行），首行 Up 才交还焦点回 Options；末行 Down no-op 且不丢焦点。
- **几何单源**（`mod.rs::input_wrap_width`）：Input wrap 宽度公式（popup 宽 → 内边 → Input 边框 2 → 预留光标 1 格）由 view 与 app.rs 共享，防止渲染/按键两侧几何漂移导致光标错位；`handle_question_key`/`route_question_key` 增加 `width` 参数（app.rs 传 `terminal.size()` 派生值，先例同 handle_key）。
- **自适应高度**（`view.rs`）：固定 `INPUT_HEIGHT=3` → `composer::display_rows` 动态行数 clamp 1..=6（`MAX_INPUT_ROWS=6`，超出内部滚动）；`popup_sections` 接收动态高度，Input 仍钉在弹窗底部、弹窗总高仍被 composer_top 封顶；`render_input` 改 `wrap_rows` 切行渲染 + 垂直滚动 + `cursor_row_col` 定位硬件光标，删除单行 `input_window`。
- **粘贴**（`paste_custom`）：`sanitize_single_line` 逐字符插入 → `composer::insert_str`（`sanitize_multiline` 保换行 + 256KiB 上限自动生效），多行粘贴语义与主输入框一致。
- **拆分**：state.rs 测试外置 `state_tests.rs`（816→499 行，守 ≤800 红线；新文件 319 ≤400）。

## Validation（当次实跑）

- `cargo test -p opencoder-tui`：**1464 passed / 0 failed**（lib 1398 + e2e 全绿；question_menu 模块 32 测试）。
- `cargo clippy --workspace --all-targets`：零警告。
- `cargo test --workspace`：全量回归通过（全绿；两轮实跑：改动后 2865 passed / 0 failed，测试拆分后复跑见工作树状态）。
- e2e `crates/tui/tests/question_flow.rs`：worker 级 question 全流程（dialog → hub resolve → Tool result）不受签名/渲染改动影响，保持通过。

## 测试覆盖表

| 测试 | 层 | 覆盖点 |
|---|---|---|
| `state_tests.rs::readline_jump_keys_reach_line_boundaries` | unit | Ctrl+A/E、Home/End 按逻辑行（非整缓冲）跳转 |
| `state_tests.rs::ctrl_u_clears_and_ctrl_k_deletes_to_the_end` | unit | Ctrl+U 清空、Ctrl+K 从光标删到末尾 |
| `state_tests.rs::word_keys_delete_and_move_by_word` | unit | Ctrl+W/Alt+Backspace 删前词、Alt+F/Alt+B 词移动 |
| `state_tests.rs::delete_key_removes_the_char_under_the_cursor` | unit | Delete 前向删除 |
| `state_tests.rs::explicit_newline_keys_keep_enter_as_confirm` | unit | Shift+Enter/Ctrl+J 换行、Enter 仍按选中项确认（多行 answer 组装） |
| `state_tests.rs::up_down_move_across_wrapped_rows_before_leaving_the_input` | unit | 软换行行内移动、首行 Up 才回 Options、末行 Down no-op |
| `state_tests.rs::up_crosses_explicit_newlines_too` | unit | 显式换行的 Up 跨行移动 |
| `state_tests.rs::paste_preserves_newlines_and_aligns_the_cursor` | unit | 多行粘贴保留 `\n`、Tab 展开为空格、光标对齐（原单行测试语义升级） |
| `view.rs::selected_row_stays_highlighted_while_the_input_has_focus` | unit（TestBackend） | focus=Custom 时选中行仍 `▸` 高亮、未选中行/Custom 行无标记 |
| `view.rs::wrapped_input_grows_the_input_box_and_tracks_the_cursor` | unit（TestBackend） | 120 字符软换行 → 框 3 内容行、光标落第三行正确列 |
| `view.rs::input_taller_than_the_cap_scrolls_to_keep_the_cursor_visible` | unit（TestBackend） | 8 行内容超 6 行 cap → 内部滚动、row1 滚出、光标仍可见 |
| `view.rs::explicit_newlines_also_grow_the_input_box` | unit（TestBackend） | 显式换行同样增高、各行文本可见、光标落末行 |
| 既有 view 测试（cursor_starts / unicode_width / wrapped_content 封顶） | unit | 单行几何不回归（坐标断言原值保持） |

无删测试 / 无 `#[ignore]` / 无弱断言（原 `paste_keeps_single_line`、`long_input_window` 为语义变更点，按新契约改写而非保留旧断言）。
