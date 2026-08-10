Commit: 0e0ec867c45170ffb244e38469baf7f4508bacc9

# Notepad 软换行边界光标语义校准

## Context

Notepad 复用 composer 的插入光标规则后，把软换行共享边界归到上一可视行。Normal `j/k` 为进入下一行而额外跳过一个字符，导致第 0 列移动到下一行时落在第 1 列，硬件光标也可能显示在上一行尾部。

## Change Summary

- Notepad 对共享软换行边界采用 Vim Normal 字符光标语义：边界字符属于下一可视行。
- 删除跨行后的 `index += 1` 补偿，垂直移动精确保留显示列。
- 显式换行符仍属于前一逻辑行，下一字符从新行第 0 列开始。

## Impact Surface

只影响 `/notepad` 的光标定位和垂直移动；composer 的插入光标规则、文件内容及保存格式不变。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| ASCII wrap 边界精确列与上下往返 | `vertical_motion_does_not_stick_on_soft_wrap_boundary` | `notepad/editor_layout.rs` |
| 显式换行边界保持逻辑行语义 | `explicit_newline_boundary_stays_on_previous_logical_line` | `notepad/editor_layout.rs` |
| 硬件光标显示在续行首列 | `render_editor_cursor_uses_next_row_at_soft_wrap_boundary` | `notepad/render_tests.rs` |

- 全量回归：`cargo test --workspace --quiet` → 2308 passed / 0 failed。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告。

## Related Docs

- [TUI 模块](../../../agents/tui/index.md)
