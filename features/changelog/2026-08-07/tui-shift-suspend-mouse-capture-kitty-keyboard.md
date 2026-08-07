# TUI Shift 挂起鼠标捕获 + Kitty 键盘协议增强

## 背景

TUI 启用了鼠标捕获（click 命中测试、双击展开 subagent），但终端的鼠标捕获
与原生文本选择互斥——用户按住 Shift 拖拽选中文本时，鼠标事件仍被 TUI 拦截，
无法复制。此外，Kitty 键盘协议 `REPORT_EVENT_TYPES` 投递的按键 Release 事件
未被过滤，可能导致同一按键被处理两次（ghost double-trigger）。

本次引入：按住 Shift 时挂起鼠标捕获（让终端原生选择），释放后恢复；并补全
Kitty 键盘协议的 Release / bare-modifier 事件过滤。

## 变更

### 1. Kitty 键盘协议 flag 补全

`crates/tui/src/terminal.rs` 的 `TerminalGuard::enter()` 与 `resume_screen()`
新增两个 flag：

- `REPORT_EVENT_TYPES` — 报告按键的 Press/Repeat/Release kind，使
  `consume_modifier_or_release` 能区分按下与释放。
- `REPORT_ALL_KEYS_AS_ESCAPE_CODES` — 报告 bare modifier（Shift/Ctrl/Alt）
  为 `KeyCode::Modifier(...)` 事件，使 Shift 按下/释放可被检测。

这两个 flag 与既有的 `DISAMBIGUATE_ESCAPE_CODES` + `REPORT_ALTERNATE_KEYS`
组合设置；不支持 Kitty 协议的终端静默忽略。

### 2. Shift 挂起/恢复鼠标捕获

`crates/tui/src/terminal.rs` 新增：

- `suspend_mouse_capture() -> Result<()>` — 发出 `DisableMouseCapture`，让终端
  原生选择文本。
- `resume_mouse_capture() -> Result<()>` — 发出 `EnableMouseCapture`，恢复点击
  交互。
- `consume_modifier_or_release(k, &mut shift_held) -> bool` — 事件过滤器，
  位于 `app.rs` 事件循环最前（`Event::Key` arm 首行），在任何其它处理之前：

  | 事件 | 行为 | 返回 |
  |------|------|------|
  | Shift Press/Repeat | `shift_held=true`，suspend capture | true（吞没）|
  | Shift Release | `shift_held=false`，resume capture | true（吞没）|
  | 其它 bare modifier（Ctrl/Alt/Super）| 不改状态 | true（吞没）|
  | 非 Shift 键 Release | 过滤，防 double-trigger | true（吞没）|
  | 普通按键 Press | 透传 | false |

  返回 `true` → `app.rs` `continue`（事件不进入 `handle_key`）。

### 3. 状态

`crates/tui/src/app.rs`：新增 `let mut shift_held = false;` 循环局部变量（与
`dbl_click` 并列）。Shift 状态不经 struct 字段、不经 worker——纯本地。

### 4. 恢复序列同步

`write_restore()` 新增发出 `DisableMouseCapture` + `PopKeyboardEnhancementFlags`
（与 `DisableBracketedPaste` + `LeaveAlternateScreen` 并列），确保终端退出时
清理所有增强。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| Shift-Left press→suspend/repeat 幂等/release→resume | `consume_modifier_toggle_on_shift_left_press_repeat_release` | `terminal.rs` |
| Shift-Right 镜像 Left | `consume_modifier_toggle_on_shift_right_press_release` | `terminal.rs` |
| Ctrl/Alt 不改 shift_held | `consume_modifier_consumes_non_shift_modifiers_without_state_change` | `terminal.rs` |
| 普通键透传、不清 held | `consume_modifier_passes_through_normal_key_press` | `terminal.rs` |
| 非 Shift Release 过滤 | `consume_modifier_filters_non_shift_key_release` | `terminal.rs` |
| suspend/resume 无 TTY 安全 | `mouse_capture_toggle_is_safe_without_tty` | `terminal.rs` |
| 恢复序列含全部 4 条 | `write_restore_emits_all_restoration_sequences` | `terminal.rs` |

- 全量回归：`cargo test --workspace` → **2017 passed / 0 failed**
- TUI lib：`cargo test -p opencoder-tui --lib` → **1013 passed / 0 failed**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → Finished，零错误

## 备注

- 单元测试覆盖 `consume_modifier_or_release` 的纯逻辑；`app.rs` 事件循环
  `continue` 接线为集成层，无自动化 e2e（TUI 无 e2e 框架）。建议在真实
  Kitty/iTerm2 终端手动验证：Shift+拖拽原生选择、释放恢复点击、无 ghost。
- Kitty flag 是 best-effort（`let _ = execute!(...)`），不支持时静默降级。
