Commit: (working-tree, pre-initial-commit)

# feat(tui): notepad 拖动方向修复 + 拖动光标变形 + Ctrl+Shift+T 收起 composer + 编辑器光标修复

## 背景

四个相关缺陷：

1. **拖动方向反转**：拖动 notepad 分隔条时方向相反——向下拖反而**缩小**高度，向上拖才增大，
   与直觉相反。
2. **拖动无光标提示**：拖动分隔条期间终端硬件光标无任何视觉反馈，用户不知当前处于拖拽态。
3. **无法收起 composer**：底部输入区常驻，无法临时隐藏以腾出信息区空间。
4. **编辑器光标被 composer 抢占**：当 notepad 文件编辑器持有焦点时，`render()` 仍调用 composer 的
   `set_cursor_position`，把硬件光标定到 composer，导致编辑器光标不可见/错位。

## 变更

- **`crates/tui/src/app_notepad.rs`**：拖动方向修复——向下拖动分隔条 = 高度**增大**（分隔条下移）。
  先前的 `5 - delta` 反向逻辑改为正向累加。
- **`crates/tui/src/frame.rs`**：每帧派生硬件光标形状（自愈式，panic/模式切换后自动恢复）——
  拖动分隔条时 `SetCursorStyle::SteadyUnderScore`（下划线），否则回到应用默认 `SteadyBar`（竖条）。
  依赖 `drag_active = np_drag.is_some()`。
- **`crates/core/src/config/keymap.rs`**：新增 `hide_composer` 字段（默认 `"ctrl+shift+t"`），
  `KEYMAP_INFO` 加标签，`get`/`set` 加分支。绑定总数 20→21。
- **`crates/tui/src/keymap.rs`**：`KeyCombo::matches` 变为 SHIFT 感知——
  显式含 SHIFT 的 spec（如 `ctrl+shift+t`）不再匹配无 SHIFT 的事件，避免与普通 `ctrl+t`
  （模式切换）碰撞；不含 SHIFT 的 spec 保持宽松以兼容终端。`KeyBindings` 新增 `hide_composer`。
  **向后兼容**：22/0 matcher 测试通过，`ctrl+shift+tab`/`alt+tab` 仍匹配，普通 `ctrl+t` 仍宽松。
- **`crates/tui/src/key_handler.rs`**：新增 `KeyAction::ToggleComposer`，在 `switch_mode_clear`
  之前检查，避免宽松 ctrl 匹配器让 `ctrl+t` 吞掉 `ctrl+shift+t`。
- **`crates/tui/src/app.rs`**：新增 `composer_collapsed` 状态（Ctrl+Shift+T 翻转），连同
  `np_drag.is_some()` 与 `notepad_editor_focused(...)` 透传给 `render_frame`。
- **`crates/tui/src/app_display.rs`**：新增 `pub(super) fn notepad_editor_focused()`（编辑器焦点判定，
  供渲染层光标门控）与 `fn render_composer_hint()`（收起态占位提示行）。
- **`crates/tui/src/render.rs`**：收起态渲染提示行（`❯ Input hidden — Ctrl+Shift+T to show`），
  隐藏草稿且光标归零；编辑器聚焦时跳过 composer 的 `set_cursor_position`，保持光标在编辑器面板内。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 拖动方向：向下增大高度 | `drag_down_increases_height` | `crates/tui/src/app_notepad.rs` |
| 拖动整体调整高度 | `drag_adjusts_height` | `crates/tui/src/app_notepad.rs` |
| 分隔条点击起始拖动 | `drag_starts_on_divider_click` | `crates/tui/src/app_notepad.rs` |
| 拖动夹紧最小高度 5 | `drag_clamps_to_minimum_5` | `crates/tui/src/app_notepad.rs` |
| SHIFT 匹配器区分 ctrl+shift 与普通 ctrl | `match_ctrl_shift_letter_distinguishes_from_plain_ctrl` | `crates/tui/src/keymap_tests.rs` |
| 收起态提示组件渲染 | `collapsed_composer_hint_widget` | `crates/tui/src/render_tests/composer.rs` |
| 收起态完整渲染：显示提示、隐藏草稿、光标归零 | `collapsed_composer_full_render_shows_hint_and_no_cursor` | `crates/tui/src/render_tests/composer.rs` |
| 编辑器聚焦时光标留在编辑器而非 composer | `editor_focus_cursor_stays_in_editor_not_composer` | `crates/tui/src/render_tests/composer.rs` |
| keymap 绑定数 = 21 | `keymap_info_count_matches_fields` | `crates/core/src/config/keymap.rs` |
| keymap 菜单条目数 = 21 | `new_menu_has_21_entries` | `crates/tui/src/keymap_menu/state.rs` |
| hide_composer 默认 = ctrl+shift+t | `default_values_match_documented_defaults` | `crates/core/src/config/keymap.rs` |

> **手测项**：拖动光标变形（frame.rs `SetCursorStyle`）属终端 I/O，无确定性单测；通过实机拖动分隔条
> 观察下划线光标 + 释放后恢复竖条验证。

## 回归

- 全量回归：`cargo test --workspace` → **2248 passed / 0 failed**（tui lib 1164 / core lib 84 / session 271 …）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（EXIT=0）
- build：`cargo build --workspace` → 零错误（Finished dev profile）
- 行数：`app.rs` 800（≤ 800，无余量）；`render.rs` 767；`app_display.rs` 230；`frame.rs` 282；
  `keymap.rs`(core) 196；`keymap.rs`(tui) 333；`app_notepad.rs` 339；`key_handler.rs` 463；
  `render_tests/composer.rs` 307；`keymap_tests.rs` 219
