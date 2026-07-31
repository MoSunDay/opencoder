Commit: (working-tree, pre-initial-commit)

# TUI 鼠标复制——双击窗口提取 + 空选择诚实反馈

## 背景

`clip_probe` 模块（commit 9bc57ca）已经让 `selection.rs` 的复制路径给出诚实的
成功/失败反馈，但 `app_helpers.rs` 的鼠标交互侧仍有两处遗留：

1. **双击判定逻辑内联且不可测**——`handle_mouse` 里直接写
   `now.duration_since(t) < Duration::from_millis(DBL_CLICK_MS)`，阈值逻辑埋在
   事件处理流程中，无法单独单元测试。
2. **空选择无反馈**——鼠标松开时若 `finish_copy` 返回 `None`（拖拽选中的全是
   空白、或双击空行），`copy_msg` 静默不变，用户不知道为什么没复制。

## 变更

### `crates/tui/src/app_helpers.rs`（796→800 行）
- **`DBL_CLICK_MS` 400 → 500**：略微放宽双击窗口，减少快速连点被判为两次
  单击的误判（796→800 行，未超 800 上限）。
- **新增 `pub(crate) fn is_within_dbl_click_window(prev, now)`**：把阈值判定
  提取为命名纯函数，`handle_mouse` 双击分支改为调用它——逻辑可独立测试。
- **mouse-Up 分支新增 "Nothing to copy at this position"**：`finish_copy` 返回
  `None` 时，仅当存在真实拖拽（`sel.0 != sel.1`）或双击（`*dbl_click`）才提示；
  裸单击（lo==hi）保持静默，不骚扰用户。

### `crates/tui/src/app_helpers_tests/mod.rs`
- 注册 `mod mouse_clip_tests;`。

### `crates/tui/src/app_helpers_tests/mouse_clip_tests.rs`（新增，398 行）
- 集成测试（经 `StubStore` + `ChatView` 全量 `handle_mouse` 调用）：
  - `shift_drag_does_not_copy_on_release` / `shift_drag_down_clears_selection_and_returns_none`
  - `multi_line_drag_copies_on_release`
  - `double_click_blank_line_shows_nothing_to_copy`
- 纯函数测试（`is_within_dbl_click_window` 阈值边界）：
  - `dbl_click_window_within_threshold` / `dbl_click_window_beyond_500ms`

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 双击窗口内为真 | `dbl_click_window_within_threshold` | mouse_clip_tests.rs |
| 超 500ms 为假 | `dbl_click_window_beyond_500ms` | mouse_clip_tests.rs |
| Shift+拖拽不触发复制 | `shift_drag_does_not_copy_on_release` | mouse_clip_tests.rs |
| 多行拖拽复制成功 | `multi_line_drag_copies_on_release` | mouse_clip_tests.rs |
| 双击空行提示无可复制 | `double_click_blank_line_shows_nothing_to_copy` | mouse_clip_tests.rs |
| 裸单击不产生消息 | `single_click_does_not_copy_on_release` | mouse_tests.rs |

- 全量回归：`cargo test --workspace` → 全绿（TUI lib 696 passed；workspace 0 failed）。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告。
- build：`cargo build --workspace` → 编译干净。
- 行数：app_helpers.rs 800 ≤ 800；mouse_clip_tests.rs 398 ≤ 400。

## Impact Surface
- 用户感知：拖拽选中空白或双击空行时，看到 "Nothing to copy at this position"
  而非静默无反馈；裸单击仍不打扰。双击窗口从 400ms 放宽到 500ms。
- 不影响：CLI/Web/session/store 边界；仅 TUI 鼠标选择的 mouse-Up 反馈路径。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
- [clip_probe 模块 changelog](tui-clipboard-probe-module.md)
