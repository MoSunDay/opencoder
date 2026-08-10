Commit: (working-tree, pre-initial-commit)

# Notepad 模式标签移至边框左下角

## Context

Notepad 的 vim 模式标签（` NORMAL ` / ` INSERT ` / `:cmd` 等）此前渲染在编辑器边框的右下角。右下角通常是滚动/统计信息的习惯位置，而模式标签属于状态前缀，放在左下角更符合常规阅读顺序；同时左下角与 tree 面板的聚焦边框标题互不干扰。

## Change Summary

- `crates/tui/src/notepad/editor.rs`：`render_editor` 的 `title_bottom` 对齐常量由 `Alignment::Right` 改为 `Alignment::Left`，模式标签固定在边框左下角。
- 注释同步更新；其余渲染逻辑、按键/光标/滚动/搜索行为零改动。

## Impact Surface

- 只影响 `/notepad` 编辑器边框的模式标签位置；composer、chat、tree 面板渲染不受影响。
- `plan_edit.rs` 右下角独立的 ` NORMAL ` 状态标签不受影响（独立渲染，不在本次改动范围）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 模式标签渲染在边框左下角（buffer 断言 x=1..8） | `render_editor_mode_label_at_bottom_left` | `notepad/render_tests.rs` |
| 右下角不再出现模式标签（旧对齐回归） | `render_editor_mode_label_at_bottom_left` | `notepad/render_tests.rs` |

- 全量回归：`cargo test --workspace --quiet` → **2309 passed / 0 failed**（145 binaries）。数字取自隔离 worktree（d5fb85e + 本改动两文件）当次实跑，排除工作树中其它在途改动的影响；工作树含其它在途改动时实测 2326 passed。
- notepad 定向回归（同隔离环境）：`cargo test -p opencoder-tui --lib notepad` → 92 passed；4 个集成 test（notepad_edit_flow 5 / notepad_file_flow 8 / notepad_scroll 9 / notepad_search_terminal 3）→ 25 passed。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告。
- build：`cargo build --workspace` → 编译干净。
- 行数：`editor.rs` 655 ≤ 800；`render_tests.rs` 259 ≤ 800。

## Related Docs

- [TUI 模块](../../../agents/tui/index.md)
