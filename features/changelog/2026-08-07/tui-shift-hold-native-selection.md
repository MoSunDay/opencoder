# TUI 按住 Shift 暂停鼠标捕获，恢复终端原生文本选择

## 背景

TUI 启动时执行 `EnableMouseCapture`，终端的鼠标事件被 opencoder 拦截用于
点击/双击交互。副作用：用户无法用鼠标在终端里**原生选中文本**（Shift 拖选
在很多终端里本可绕过鼠标捕获，但 opencode 的 Kitty 键盘增强把按键事件也吃
掉了，Shift 的按下/释放事件从未到达选择逻辑）。

用户期望：**按住 Shift → 暂停鼠标捕获 → 终端原生选文；松开 Shift → 恢复
鼠标捕获**，且不破坏既有点击/双击行为。

## 变更

### 1. 启用 Kitty `REPORT_EVENT_TYPES` + `REPORT_ALL_KEYS_AS_ESCAPE_CODES`

`crates/tui/src/terminal.rs` 的 `TerminalGuard::enter` 与 `resume_screen` 两处
`PushKeyboardEnhancementFlags` 新增两个 flag：

- `REPORT_EVENT_TYPES` — 让 Shift 修饰键产生独立的 Press/Repeat/Release 事件
  （默认 Kitty 协议只报 Press，无法感知"松开"）。
- `REPORT_ALL_KEYS_AS_ESCAPE_CODES` — 确保修饰键本身也作为事件上报。

两处同步修改，保证 resume 后行为一致。终端不支持 Kitty 协议时这些 flag 被忽略
（best-effort）。

### 2. 鼠标捕获挂起/恢复

新增 `suspend_mouse_capture()` / `resume_mouse_capture()`（终端 best-effort，
错误被忽略），分别执行 `DisableMouseCapture` / `EnableMouseCapture`。

### 3. `consume_modifier_or_release` 状态机

`crates/tui/src/terminal.rs` 新增纯函数 `consume_modifier_or_release(k, shift_held)`：

- **Shift 按下/重复**：首次按下时设 `shift_held=true` 并 `suspend_mouse_capture`；
  重复按下幂等（不重复发序列）。返回 `true`（消费事件）。
- **Shift 释放**：若已 held，置 `shift_held=false` 并 `resume_mouse_capture`。
  返回 `true`。
- **其它裸修饰键**（Ctrl/Alt/Super）：返回 `true`（消费，不触发 app 动作），
  不改动 `shift_held`。
- **普通按键**（如 `a`）：返回 `false`（交给 app 正常处理）。
- **任意非 Shift 键的 Release**：返回 `true`（过滤，避免 REPORT_EVENT_TYPES
  导致的重复触发）。

### 4. 接入 app 主循环

`crates/tui/src/app.rs`：

- 新增 `let (mut dbl_click, mut shift_held) = (false, false);`（与既有
  `dbl_click` 合并声明，避免新增独立行）。
- `Event::Key(k)` 处理入口第一时间调用
  `if consume_modifier_or_release(&k, &mut shift_held) { continue; }`，
  修饰键/Release 在到达既有点击/双击逻辑前被拦截。

## 测试覆盖

5 个 `consume_modifier_or_release` 行为单元测试（内联于 `terminal.rs`，纯函数、
零 I/O/网络/DB，<10ms），每条同时断言**返回值**与 `shift_held` 状态变迁：

| 分支 | 测试名 | 文件 |
|------|--------|------|
| Shift Left 按→重复→释放 | `consume_modifier_toggle_on_shift_left_press_repeat_release` | `crates/tui/src/terminal.rs` |
| Shift Right 按→释放 | `consume_modifier_toggle_on_shift_right_press_release` | `crates/tui/src/terminal.rs` |
| Ctrl/Alt 裸修饰键消费、不改状态 | `consume_modifier_consumes_non_shift_modifiers_without_state_change` | `crates/tui/src/terminal.rs` |
| 普通键 `a` 透传、不碰状态 | `consume_modifier_passes_through_normal_key_press` | `crates/tui/src/terminal.rs` |
| 非 Shift 键 Release 过滤 | `consume_modifier_filters_non_shift_key_release` | `crates/tui/src/terminal.rs` |

- 全量回归（工作树）：`cargo test --workspace` → **2017 passed / 0 failed / 0 ignored**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → Finished，零错误

## 未覆盖（residual）

Kitty flag 的实际效果（按住 Shift 真的能原生选文）依赖真实 Kitty 终端，属 e2e
范畴，单测只覆盖纯函数契约。无 Kitty 终端环境 → e2e soft-skip。
