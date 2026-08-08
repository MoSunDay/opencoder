# TUI KeymapMenu 按钮栏 + 帮助弹窗恢复

## 背景

Ctrl+H 打开的 KeymapMenu（快捷键重绑定弹窗）此前只有底部提示行，没有可点击/可聚焦
的按钮。旧版本删除的静态帮助弹窗（`help.rs` / `keybind.rs`）内容仍有价值——用户需要
一份完整的快捷键参考。本次在 KeymapMenu 底部新增按钮栏（退出 / 帮助），并将旧帮助
内容恢复为可滚动的帮助覆盖层。

## 变更

### 1. 新增 help 模块：`keymap_menu/help.rs`（176 行，新文件）

- 从已删除的 `keybind.rs` 恢复 `HELP` 常量（完整中文快捷键指南），更新
  `Ctrl+H` 条目描述为「快捷键设置面板（查看 / 重绑快捷键，含「帮助」按钮）」。
- 从已删除的 `help.rs` 恢复 `wrap_line`（CJK 感知换行）、`build_wrapped_lines`。
- 新增 `render_help_overlay(f, area, scroll)`：居中弹窗，标题
  `帮助 (Esc 关闭, ↑↓ 滚动)`，使用 `rounded_block_focus`（accent 边框）。
- 7 个单元测试：wrap_line 各路径 + HELP 内容断言。

### 2. state.rs 新增焦点 + 按钮栏状态

- 新增 `Focus` 枚举：`List`（快捷键列表）/ `Buttons`（底部按钮栏）。
- `KeymapMenu` 新增字段：`focus`、`selected_button`（0=Exit, 1=Help）、
  `help_open`、`help_scroll`；配套 pub 访问器。
- `handle_keymap_key` 重构为三段式：
  1. **Help 覆盖层打开**：↑/↓ 滚动，Esc 关闭覆盖层（不关闭弹窗）。
  2. **Capture 模式**：逻辑不变。
  3. **全局快捷键**（Tab 切焦点 / Esc / Ctrl+D）+ **焦点相关分支**
    （List: 现有导航 + rebind；Buttons: ←/→/Ctrl+J/K 选择按钮，Enter 激活）。
- `close_with_save` 辅助函数：Exit 按钮 = Ctrl+D 同行为（dirty→Save 否则→Quit）。
- 12 个新单元测试：Tab 切焦点、按钮导航、Exit/Help 激活、Help 开/关/滚动。

### 3. view.rs 新增按钮栏渲染

- 弹窗高度 +1（`3 + rows + 2`）容纳按钮栏行。
- 按钮渲染 `< 退出 >` / `< 帮助 >`，选中按钮 accent + BOLD 高亮。
- 列表焦点不在 List 时，`❯` 标记 dim 化，避免视觉歧义。
- `help_open` 时在 KeymapMenu 之上渲染 `render_help_overlay`。

### 4. mod.rs 导出 help 模块

- 新增 `pub mod help;`。

## 测试覆盖

| 功能 | 测试名 | 文件 | 层 |
|------|--------|------|-----|
| wrap_line 短文本透传 | `wrap_line_short_passthrough` | `keymap_menu/help.rs` | unit |
| wrap_line 空格断行 | `wrap_line_breaks_at_space` | `keymap_menu/help.rs` | unit |
| wrap_line 硬断行 | `wrap_line_long_word_hard_break` | `keymap_menu/help.rs` | unit |
| wrap_line CJK 感知 | `wrap_line_cjk_aware` | `keymap_menu/help.rs` | unit |
| wrap_line 空字符串 | `wrap_line_empty` | `keymap_menu/help.rs` | unit |
| build_wrapped_lines 非空 | `build_wrapped_lines_nonempty` | `keymap_menu/help.rs` | unit |
| HELP 文本更新 | `help_mentions_keymap_panel` | `keymap_menu/help.rs` | unit |
| Tab List→Buttons | `tab_toggles_focus_list_to_buttons` | `keymap_menu/state.rs` | unit |
| Tab Buttons→List | `tab_toggles_focus_buttons_to_list` | `keymap_menu/state.rs` | unit |
| ←/→ 按钮导航 | `left_right_navigate_buttons` | `keymap_menu/state.rs` | unit |
| Ctrl+J/K 按钮导航 | `ctrl_j_ctrl_k_navigate_buttons` | `keymap_menu/state.rs` | unit |
| Exit 按钮退出 | `exit_button_quits_without_changes` | `keymap_menu/state.rs` | unit |
| Exit 按钮 dirty→Save | `exit_button_saves_when_dirty_then_quits` | `keymap_menu/state.rs` | unit |
| Help 按钮开覆盖层 | `help_button_opens_overlay` | `keymap_menu/state.rs` | unit |
| Help 滚动↓ | `help_overlay_scroll_down_increments` | `keymap_menu/state.rs` | unit |
| Help 滚动↑ | `help_overlay_scroll_up_decrements` | `keymap_menu/state.rs` | unit |
| Esc 关 Help 覆盖层 | `help_overlay_esc_closes` | `keymap_menu/state.rs` | unit |
| Esc 不关弹窗 | `help_overlay_esc_does_not_close_modal` | `keymap_menu/state.rs` | unit |
| ↑/↓ 回到列表 | `up_down_in_buttons_focus_returns_to_list` | `keymap_menu/state.rs` | unit |

- TUI 回归：`cargo test -p opencoder-tui` → **1108 passed / 0 failed**
- keymap_menu 专项：30 passed（7 help + 23 state）
- clippy：`cargo clippy -p opencoder-tui --all-targets -- -D warnings` → 零警告
- build：`cargo build -p opencoder-tui` → Finished，零错误
