# TUI 可重绑定键盘快捷键（keymap / /short_key）

## 背景

此前所有 TUI 快捷键（quit/cancel/help/newline/cursor 移动/undo/redo/模式
切换/collapse/force-redraw 等 18 个动作）硬编码在 `key_handler.rs` 的
`KeyCode`/`KeyModifiers` 比较中，用户无法自定义。本次引入可配置的 keymap
层：用户可在 `opencoder.json` 的 `"keymap"` 段重绑定任意快捷键，或通过 TUI
内 `/short_key`（别名 `/sk`）弹出菜单交互式捕获新绑定并即时保存、热重载。

## 变更

### 1. 配置层（core crate）

- **`crates/core/src/config/keymap.rs`**：`KeymapConfig`（18 个 `String`
  字段）+ `KEYMAP_INFO`（config_key ↔ human_label 对照表）。`Default` 硬编码
  18 个默认 spec；`get`/`set` 字符串键访问器供 merge 与菜单使用。
- **`Config.keymap`** 字段（`config.rs`，`#[serde(default)]`）。
- **`merge.rs`**：`merge_into` 遍历 `"keymap"` 对象逐字段 `cfg.keymap.set()`；
  `has_editable_key()` 将非空 keymap 计为可编辑配置。

### 2. 解析/匹配层（tui crate）

- **`crates/tui/src/keymap.rs`**：`KeyCombo { mods, code }` + `KeyBindings`
  （18 个解析后的 combo，`from_config(&Config)` 构建，坏 spec 回退默认）。
- **`parse_key_spec`**：解析 `"ctrl+h"`、`"alt+tab"`、`"ctrl+shift+tab"` 等；
  `shift+tab` 归一化为 `BackTab`（避免冗余 SHIFT flag）。
- **`KeyCombo::matches`**：终端兼容匹配——
  - BackTab 归一化（`shift+tab` ≡ `backtab`）；
  - Tab/BackTab SHIFT-optional（`alt+tab` 同时匹配真实 `Tab+ALT` 和
    `BackTab+ALT`）；
  - raw 控制字符匹配（Kitty 协议将 `Ctrl+D` 投递为 `\u{4}`，带或不带
    CONTROL flag 均匹配）；
  - alt-combo 大小写不敏感。
- **`key_event_to_spec`**：逆操作，捕获模式将 `KeyEvent` 转回 spec 字符串。

### 3. 菜单 UI 层

- **`keymap_menu/state.rs`**：`KeymapMenu`（选中行 `selected`、捕获模式
  `capturing`、18 行 entries、原 spec 快照用于脏检测）+ `KeymapOutcome`
  （Idle/Cancel/Save(patch)/Quit）。`handle_keymap_key` 处理导航（↑/↓ 环绕、
  Ctrl+J/K）、Enter 进入捕获、Esc/Ctrl+D 保存或退出。`build_patch()` 仅返回
  变更字段。
- **`keymap_menu/view.rs`**：`render_keymap_popup` 居中渲染圆角边框模态，18 行
  （`❯ | spec | label`），捕获态显示 "Press a key..."。
- **`command.rs`**：`SlashAction::ShortKey`；`try_keymap_command` 解析
  `/short_key`、`/sk`（trim 容忍前后空格）。

### 4. 接线

- **`app.rs`**：`keymap_menu` 状态；菜单打开时拦截所有键盘事件到
  `handle_keymap_outcome`；paste 路由在菜单打开时阻断。
- **`app_loop.rs`**：`dispatch_command` 的 `ShortKey` arm 创建
  `KeymapMenu::new(&config.keymap)`；`handle_keymap_key` Save 分支 →
  `Config::save(workdir, &patch)` → `Config::load(workdir)` → 重建
  `KeyBindings::from_config`，新绑定即时生效。
- **`render.rs`/`frame.rs`**：`render()` 签名新增 `keymap_menu` 参数，渲染
  popup。
- **`key_handler.rs`/`app_helpers.rs`**：18 个动作从硬编码 `KeyCode` 比较改为
  查询 `KeyBindings` 字段的 `.matches(&k)`。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 解析 ctrl/alt/shift/fn/bare | `parse_ctrl_letter` 等 8 项 | `keymap_tests.rs` |
| 匹配精确/raw-ctrl/大小写/backtab | `match_exact` 等 7 项 | `keymap_tests.rs` |
| key_event_to_spec 往返 | `roundtrip_ctrl_letter` 等 3 项 | `keymap_tests.rs` |
| from_config 默认/自定义/回退 | `from_config_*` 3 项 | `keymap_tests.rs` |
| 菜单 18 行/导航环绕/捕获/保存/退出 | `new_menu_has_18_entries` 等 9 项 | `keymap_menu/state.rs` |
| config 默认/get/set/info 对齐 | 4 项 | `config/keymap.rs` |
| /short_key 解析/dispatch/菜单 picker | `parse_short_key` 等 3 项 | `command.rs` |

- 全量回归：`cargo test --workspace` → **2017 passed / 0 failed**
- TUI lib：`cargo test -p opencoder-tui --lib` → **1013 passed / 0 failed**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → Finished，零错误

## 默认绑定

| 动作 | 默认 | 动作 | 默认 |
|------|------|------|------|
| help | ctrl+h | undo | ctrl+z |
| quit | ctrl+d | redo | ctrl+y |
| cancel | ctrl+c | forward_word | alt+f |
| newline | ctrl+j | backward_word | alt+b |
| cursor_home | ctrl+a | switch_mode_clear | alt+tab |
| cursor_end | ctrl+a→e | switch_mode_keep | ctrl+shift+tab |
| delete_word | ctrl+w | collapse_blocks | ctrl+l |
| clear_input | ctrl+u | force_redraw | ctrl+f |
| switch_mode | ctrl+t | paste_image | ctrl+v |
