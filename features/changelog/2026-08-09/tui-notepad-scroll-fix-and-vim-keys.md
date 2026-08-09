# fix(tui): notepad 编辑器滚动 bug 修复 + vim 页面滚动键 + `:e` 打开文件

## 背景

`EditorState::ensure_cursor_visible()` 是死代码——定义后从未被调用。普通光标移动
（`j`/`k`/`G`/`gg`/插入）**永远不会**更新 `editor.scroll`，导致 `render_editor` 基于
`state.scroll` 切片时永远停留在第一屏，用户无法向下滚动查看更多内容。

## 变更

### 核心修复（滚动 bug）
- **`crates/tui/src/notepad/keys.rs`**：
  - 新增 `editor_inner_height()` —— 镜像 `layout_split` 的布局约束计算编辑器内部高度。
  - `handle_editor_key` 中，在 `vim::handle_vim_key(...)` 之后调用 `ensure_cursor_visible(inner_h)`，
    使 `state.scroll` 始终与光标位置同步（vim 风格"光标跟随、边缘粘滞"）。
- **`crates/tui/src/notepad/editor.rs`**：
  - 修复 `ensure_cursor_visible`：用正确的独立 `char_byte_offset` 替换有 off-by-one bug 的方法；
    新增末页钳位防止 scroll 超出总行数。
  - 删除死代码方法 `char_byte_offset(&self)`（原有的 off-by-one 实现）。

### 增强 A：vim 页面滚动键
- `keys.rs` 新增 `try_page_scroll()` —— Normal 模式下拦截 `Ctrl-D`/`Ctrl-U`（半页）、
  `Ctrl-F`/`Ctrl-B`（全页）、`PageDown`/`PageUp`。Insert 模式不受影响。
- `editor.rs` 新增 `cursor_line()`、`move_to_line()`、`page_down()`、`page_up()` 方法。

### 增强 B：`:e {path}` 打开文件
- `editor.rs` 新增 `edit_cmd_path()`（解析 `:e`/`:edit {path}`）和 `do_edit()`（解析相对路径并加载）。
- `keys.rs` 在 `handle_editor_key` 中拦截 Enter + `:e`/`:edit` 命令，焦点保持 Editor。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 光标在视口内时 scroll 不变 | `ensure_cursor_no_change_when_visible` | `editor.rs` (unit) |
| 光标移过底部时 scroll 跟进 | `ensure_cursor_scrolls_down_when_past_bottom` | `editor.rs` (unit) |
| 光标移到顶部以上时 scroll 回退 | `ensure_cursor_scrolls_up_when_above_top` | `editor.rs` (unit) |
| scroll 钳位到末页 | `ensure_cursor_clamps_to_last_page` | `editor.rs` (unit) |
| cursor_line 在文本末尾正确 | `cursor_line_correct_at_end_of_text` | `editor.rs` (unit) |
| move_to_line 钳位 | `move_to_line_clamps` | `editor.rs` (unit) |
| page_down 移动半页 | `page_down_moves_half` | `editor.rs` (unit) |
| page_up 钳位到 0 | `page_up_clamps_to_zero` | `editor.rs` (unit) |
| `:e {path}` 解析 | `edit_cmd_path_parses_e_with_arg` | `editor.rs` (unit) |
| `:edit {path}` 解析 | `edit_cmd_path_parses_edit_with_arg` | `editor.rs` (unit) |
| 裸 `:e` 返回空串 | `edit_cmd_path_bare_e_returns_empty` | `editor.rs` (unit) |
| 非 edit 命令拒绝 | `edit_cmd_path_rejects_non_edit` | `editor.rs` (unit) |
| do_edit 加载相对路径 | `do_edit_loads_relative_path` | `editor.rs` (unit) |
| do_edit 无参数重开当前文件 | `do_edit_reopens_current_file_when_no_arg` | `editor.rs` (unit) |
| `G` 推进 scroll | `big_g_advances_scroll` | `notepad_scroll.rs` (integration) |
| `gg` 重置 scroll 到 0 | `gg_resets_scroll_to_zero` | `notepad_scroll.rs` (integration) |
| `j` 连续按下推进 scroll | `j_advances_scroll_incrementally` | `notepad_scroll.rs` (integration) |
| Ctrl-D 下移光标 | `ctrl_d_moves_cursor_down` | `notepad_scroll.rs` (integration) |
| Ctrl-U 上移光标 | `ctrl_u_moves_cursor_up` | `notepad_scroll.rs` (integration) |
| Ctrl-F 全页下移 | `ctrl_f_full_page_down` | `notepad_scroll.rs` (integration) |
| `:e` 打开文件 | `edit_command_opens_file` | `notepad_scroll.rs` (integration) |
| `:edit` 打开文件 | `edit_command_opens_with_edit_keyword` | `notepad_scroll.rs` (integration) |
| Insert 模式不触发页面滚动 | `page_scroll_does_not_fire_in_insert_mode` | `notepad_scroll.rs` (integration) |

- 全量回归：`cargo test --workspace` → **2273 passed / 0 failed**
- 隔离回归（opencoder-tui）：`cargo test -p opencoder-tui` → **1175 lib + 全部 integration passed / 0 failed**
- 新滚动测试：`cargo test -p opencoder-tui --test notepad_scroll` → **9 passed / 0 failed**
- clippy：`cargo clippy -p opencoder-tui --all-targets -- -D warnings` → 零警告
- build：`cargo build -p opencoder-tui` → 零错误
- 行数：`editor.rs` 606（≤ 800）；`keys.rs` 440（≤ 800）；`notepad_scroll.rs` 203（≤ 400，新增）
