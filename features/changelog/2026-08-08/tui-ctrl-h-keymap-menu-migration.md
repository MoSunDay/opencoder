# TUI 快捷键入口迁移：Ctrl+H → 打开 KeymapMenu（替代旧帮助弹窗 + /short_key）

## 背景

快捷键设置入口此前为斜杠命令 `/short_key`（别名 `/sk`），而 Ctrl+H 打开的是
静态帮助文本弹窗（`help.rs` / `keybind.rs`）。用户期望：Ctrl+H 直接打开精致的
keymap 重绑定弹窗（原 `/short_key` 的 UI），同时优化捕获时保留说明、新增恢复默认。

## 变更

### 1. 入口迁移：Ctrl+H → OpenKeymap

- **`key_handler.rs`**：`KeyAction` 新增 `OpenKeymap` 变体；两处
  `bindings.help.matches(&k)` 不再 toggle `show_help`，改为 `return KeyAction::OpenKeymap`。
- **`app.rs`**：`handle_key` 结果 match 新增 `OpenKeymap` arm → 创建
  `KeymapMenu::new(&config.keymap)`。
- **`app.rs`**：删除 `try_keymap_command` 调用分支，Submit 路径直接走 `local_cmd::run`。

### 2. 删除 /short_key 斜杠命令

- **`command.rs`**：删 `COMMANDS` 中 `/short_key` 行、`SlashAction::ShortKey` 变体、
  `parse()` 的 `"short_key"|"sk"` 臂、`dispatch()` 的 `"/short_key"` 臂；删 3 个旧测试，
  新增 `short_key_command_removed` 断言 `/short_key`/`/sk`/`short_key` 均返回 None。
- **`app_loop.rs`**：删 `dispatch_command` 的 `ShortKey` arm、整个 `try_keymap_command` 函数。

### 3. 清理旧帮助弹窗（彻底删除）

- 删文件：`help.rs`（141 行）、`keybind.rs`（36 行）。
- **`lib.rs`**：删 `pub mod help;`、`pub mod keybind;`。
- **`key_handler.rs`**：删 `show_help`/`help_scroll` 参数、帮助滚动拦截块、Esc 的
  `if *show_help` 分支。
- **`render.rs`**：删 `show_help`/`help_scroll` 参数及 `crate::help::render_help` 调用。
- **`frame.rs`**：同步删 `show_help`/`help_scroll` 参数及转发。
- **`app.rs`**：删 `show_help`/`help_scroll` 声明及所有传参。
- **`welcome.rs`**：文案 `Ctrl+H 查看完整快捷键列表` → `Ctrl+H 打开快捷键设置`。
- **`crates/core/src/config/keymap.rs`**：`KEYMAP_INFO` label `Toggle help popup` →
  `Open shortcut settings`（config key `help` 保留不变）。
- 测试同步：`key_handler_tests.rs`（删 2 个帮助滚动测试 + 18 处参数）、
  `key_handler_plan_edit_tests.rs`（8 处）、`key_handler_queue_scroll_tests.rs`（4 处）、
  `app_tests/mod.rs`（4 个 wrapper）、`app_tests/skill_tests.rs`（2 处）、
  `render_clear_tests.rs`、`render_tests/chips.rs`、`render_tests/arrow_click.rs`。

### 4. 捕获时保留说明

- **`keymap_menu/view.rs`**：捕获模式不再 `spans.clear()`，仅替换 spec 列（`spans[1]`）
  为 `"Press a key...  "`，marker 和 label 原样保留。

### 5. 新增恢复默认

- **`keymap_menu/state.rs`**：新增 `reset_to_defaults()` 方法（遍历 entries，逐个重置为
  `KeymapConfig::default()`）；导航模式 `Ctrl+R` 触发 reset。
- **`keymap_menu/view.rs`**：footer 更新为
  `Enter: rebind   Ctrl+R: reset to default   Esc: close   Ctrl+D: quit`。

### 6. ESC 退出设置页

- 现有逻辑已满足：导航态 ESC → 检查 dirty → Save 或 Cancel 后关闭弹窗。

## 测试覆盖

| 功能 | 测试名 | 文件 | 层 |
|------|--------|------|-----|
| reset_to_defaults 恢复默认 | `reset_to_defaults_restores_original` | `keymap_menu/state.rs` | unit |
| Ctrl+R 触发 reset | `ctrl_r_resets_to_defaults` | `keymap_menu/state.rs` | unit |
| /short_key 不再解析 | `short_key_command_removed` | `command.rs` | unit |

- 全量回归：`cargo test --workspace` → **2023 passed / 0 failed**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → Finished，零错误
