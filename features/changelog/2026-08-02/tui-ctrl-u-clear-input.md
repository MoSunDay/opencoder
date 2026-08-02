# TUI 主输入框新增 Ctrl+U：清空整个输入行（readline unix-line-discard）

## Summary

主输入框支持 `Ctrl+U`（readline 的 unix-line-discard 语义）：清空当前输入行的
全部内容并将光标重置到行首。清空操作写入 undo 快照，可被 `Ctrl+Z` 撤销——与
既有 `Ctrl+W`（删除光标前的单词）行为一致。

此前 `Ctrl+U` 仅在弹窗（model menu / help / config）内有效；主输入框中按
`Ctrl+U` 无任何效果。本次补齐主输入框的按键分发，让 `Ctrl+U` 在主输入框中
也符合 readline 习惯。

## Changes

### `crates/tui/src/key_handler.rs`
- 在 `Ctrl` 修饰符 match 内新增 `KeyCode::Char('u') | KeyCode::Char('\u{15}')` arm：
  当 `input` 非空时执行 `input.clear()` + `cursor_idx = 0` + `undo::snapshot`，
  返回 `KeyAction::None`。空输入时直接 no-op（不产生 undo 快照）。
- arm 位置：在 `Ctrl+W` 之后、`Ctrl+T` 之前，不影响其他 fallthrough。

### `crates/tui/src/keybind.rs`
- `HELP` 常量新增一行 `Ctrl+U 清空整个输入行（可被 Ctrl+Z 撤销）`。

### `crates/tui/src/app_tests/key_tests.rs`
- 新增 2 个 unit 测试（正常路径 + 边界路径）。

## 测试覆盖

| 功能 | 测试名 | 文件 | 分层 |
|------|--------|------|------|
| Ctrl+U 清空非空输入行（正常路径） | `ctrl_u_clears_entire_input_line` | `crates/tui/src/app_tests/key_tests.rs` | unit |
| Ctrl+U 对空输入为 no-op（边界路径，`if !input.is_empty()` 守卫） | `ctrl_u_on_empty_input_is_noop` | `crates/tui/src/app_tests/key_tests.rs` | unit |

## 全量回归

- TUI lib：`cargo test -p opencoder-tui --lib` → **814 passed / 0 failed / 0 ignored**
- 全量：`cargo test --workspace` → **1634 passed / 0 failed / 1 ignored**
  （1 ignored = `research_smoke_bing_wikipedia`，需真实 Chrome + 网络，环境门控，与本改动无关）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
- 行数：`key_handler.rs` 484 / `keybind.rs` 34 / `key_tests.rs` 751（均 ≤800）

## 备注

- 弹窗内的 `Ctrl+U`（model menu / help / config）行为不变——各弹窗有独立的
  match 路径，主输入框的新 arm 不影响它们。
- e2e 不强制：本次改动仅 TUI 按键分发，不触及 session runner / store 数据形状 /
  prompt 契约。
