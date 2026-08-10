Commit: 0e0ec867c45170ffb244e38469baf7f4508bacc9

# Notepad 长行按 Vim 语义软换行

## Context

Notepad 编辑器此前把超过编辑区宽度的逻辑行直接截断，后半段内容既不可见，也无法通过滚动恢复；光标与滚动仍按逻辑行计算，与共享 Vim 引擎的可视行移动模型不一致。

## Change Summary

- 长逻辑行改为按编辑区实际文本宽度完整软换行；首行显示真实行号，续行保留空白 gutter。
- 新增纯函数可视布局，使渲染、光标定位、Normal `j/k`、页面移动、搜索跳转和垂直滚动共享同一可视行模型。
- 编辑区宽度统一从实际面板、边框和动态行号位数推导，文件树显示/隐藏及三位数以上行号不再产生输入与渲染偏差。
- 逻辑行数改为换行符数量加一，保留 Vim 对尾随换行后空白行的认知。

## Impact Surface

- 仅影响 TUI `/notepad` 的文件显示与导航；文件内容、保存格式、Store、会话上下文和其他编辑器不变。
- 未新增配置、环境变量、数据库结构或外部接口。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 长行完整软换行且 gutter 稳定 | `render_editor_soft_wraps_long_line_without_truncation` | `notepad/render_tests.rs` |
| ASCII/CJK 可视行布局 | `long_line_wraps_without_losing_text`、`cjk_wrap_and_cursor_use_display_width` | `notepad/editor_layout.rs` |
| Normal `j` 跨软换行 | `editor_j_moves_across_soft_wrapped_rows` | `notepad/keys.rs` |
| 搜索打开与翻页路径 | `search_finds_and_opens`、`page_down_moves_half` | `notepad/keys.rs`、`notepad/editor.rs` |

- 全量回归：`cargo test --workspace --quiet` → 2308 passed / 0 failed。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告。

## Related Docs

- [TUI 模块](../../../agents/tui/index.md)
