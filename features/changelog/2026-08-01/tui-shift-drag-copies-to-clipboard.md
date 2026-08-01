Commit: (working-tree, pre-initial-commit)

# TUI SHIFT+拖拽走应用复制管线（确定性写入剪贴板）

## 背景

`handle_mouse` 对 Shift+左键按下/拖拽做直接放行（清空选区后立即返回），寄希望于
终端原生选择把文字放进剪贴板。但事件一旦到达应用，说明终端已放弃原生选择
（不支持 shift-override 的终端会转发带 Shift 的事件，此时终端不会自选；X11 上
原生选择通常只进 PRIMARY 而非 CLIPBOARD；tmux 透传同样依赖外层终端），结果
剪贴板永远为空——`shift_drag_down_clears_selection_and_returns_none` /
`shift_drag_does_not_copy_on_release` 两个测试把这个缺陷固化成了"预期行为"。

## 变更

### `crates/tui/src/app_helpers.rs`（800→790 行）
- **删除 shift-bypass 块**（约 16 行）：Shift+Down/Drag 不再直接放行，落入与
  普通拖拽同一条选区路径。
- **`Up(Left)` 分支**：`finish_copy` 的 `force` 改为 `*dbl_click || SHIFT`——shift+
  单击复制光标所在行，shift+拖拽复制范围；"Nothing to copy" 条件同步加上 shift，
  空行 shift 操作给出诚实提示。

### `crates/tui/src/selection.rs`
- **`status_message()` 文案修正**：成功消息去掉 "— Shift+drag = terminal selection"
  误导后缀；SSH 失败提示改为诚实告知"终端拦截了 OSC52，需在终端/tmux 开启
  OSC52"；通用失败提示去掉 "or use Shift+drag"。同步更新 3 个 CopyReport 测试
  断言（成功两例加 `!contains("Shift+drag")`，SSH 一例改断言 OSC52 提示）。

### `crates/tui/src/keybind.rs`
- 帮助文案 "SHIFT+拖拽 = 终端原生选择（OSC52 被拦截时的备用方案）" → "SHIFT+拖拽 =
  选中并复制到剪贴板"。

### `crates/tui/src/app_helpers_tests/`
- **`mouse_helpers.rs`（新增，133 行）**：抽出 `StubStore` / `empty_hits` /
  `view_from_lines` 共享测试夹具，供 mouse_clip / mouse_dbl_click 两个模块复用。
- **`mouse_clip_tests.rs`（398→250 行）**：重写两个 shift 测试为
  `shift_drag_starts_selection` / `shift_drag_copies_on_release`，新增
  `shift_click_copies_single_line`；更新模块注释与分区标题。
- **`mouse_dbl_click_tests.rs`（新增，91 行）**：双击空行 + `is_within_dbl_click_window`
  纯函数测试迁入（为遵守 400 行上限而拆分）。
- **`mod.rs`**：注册 `mouse_helpers` / `mouse_dbl_click_tests`。

## 行为变化

- 收到 Shift 事件的终端（不支持 override）：shift+拖拽 / shift+单击**确定性复制到
  剪贴板**（OSC52 + wl-copy/xclip/xsel/pbcopy/clip.exe 同一条管线）。
- 支持原生 override 的终端（kitty/GNOME Terminal/Alacritty 等）：shift 事件不进
  应用，行为零变化。
- SSH + 拦截 OSC52 的终端：复制依旧无解，但失败提示诚实说明原因，不再误导。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| Shift+Down 锚定选区 | `shift_drag_starts_selection` | mouse_clip_tests.rs |
| Shift+Drag 扩展选区、松开复制 | `shift_drag_copies_on_release` | mouse_clip_tests.rs |
| Shift+单击复制整行（force 路径） | `shift_click_copies_single_line` | mouse_clip_tests.rs |
| 普通拖拽复制 | `multi_line_drag_copies_on_release` | mouse_clip_tests.rs |
| 双击空行诚实提示 | `double_click_blank_line_shows_nothing_to_copy` | mouse_dbl_click_tests.rs |
| 双击窗口边界 | `dbl_click_window_within_threshold` / `dbl_click_window_beyond_500ms` | mouse_dbl_click_tests.rs |
| CopyReport 文案（本地工具/OSC52/SSH/通用） | `copy_report_status_*`（5 例） | selection.rs |

## 回归

- `cargo test -p opencoder-tui --lib`：722 passed，0 failed（含 3 个新 shift 测试；基线 696 + 本功能净 +1，其余为同期其它 WIP 新增测试）。
- `cargo test --workspace`：当次 aggregate 1519 passed + 1 failed（flaky，测试名未捕获；随后 39/39 测试二进制直接复跑全部 exit 0，未复现）。
- `cargo clippy --workspace --lib -- -D warnings`：零警告。
- `cargo clippy -p opencoder-tui --lib --tests -- -D warnings`：零警告。
- `cargo clippy --workspace --all-targets -- -D warnings` / `cargo test --workspace`：当次被范围外 `crates/session/tests/resume_replay.rs:674` 未闭合定界符阻断（外部并发会话编辑中），非本功能引入。
