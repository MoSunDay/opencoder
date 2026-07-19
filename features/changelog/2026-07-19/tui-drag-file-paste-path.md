# TUI：拖入文件回显完整路径

## 背景

在 TUI 输入框（composer）里拖入一个文件时，终端会把文件路径作为一次粘贴投递进来。但此前 OpenCoder TUI 从未启用 bracketed paste 模式，`Event::Paste` 落入 `match ev` 的 `_ => {}` 被静默丢弃；实际粘贴内容是按字符逐个到达、每个走一次 `insert_char` 的 `Event::Key(Char(c))`。结果：既没有原子性的「粘贴事件」可供识别路径，拖入文件也得不到任何回显——用户得手敲完整路径。

需求：**把文件拖进输入框，自动回显该文件的完整绝对路径**；同时普通文本粘贴保持原样、不被意外改写。

## 变更

### 启用 bracketed paste 生命周期（`crates/tui/src/terminal.rs`）

- `TerminalGuard::enter()` 的 `execute!` 在 `EnableMouseCapture` 之后追加 `EnableBracketedPaste`。这样一次拖文件/粘贴以单个原子 `Event::Paste(String)` 到达（而非逐字符）。
- `write_restore()` 在 `DisableMouseCapture` 与 `LeaveAlternateScreen` 之间追加 `DisableBracketedPaste.write_ansi`，保证退出时（含 panic）还原干净，不会留下「bricked terminal」。
- 测试 `write_restore_emits_all_three_sequences` 改名为 `write_restore_emits_all_restoration_sequences`，断言扩充为四条序列（含 disable-bracketed-paste）。

### 路径解析纯函数 `paste_payload`（`crates/tui/src/app_helpers.rs`）

`pub(crate) fn paste_payload(payload: &str, workdir: &Path) -> String`：

- 去掉末尾单个换行（很多终端粘贴时附带的 `\n`/`\r`）。
- 多行 / 空内容原样返回（普通多段文本粘贴不被改写）。
- 剥离首尾单/双引号、`file://` URI 前缀。
- `resolve_existing_path(candidate, workdir)`：绝对路径直接 `canonicalize()`；相对路径 `workdir.join(candidate)` 后再 `canonicalize()`（所以拖入 `src/main.rs` 这样的裸相对文件名也能解析成完整绝对路径）。失败时回退一次反斜杠转义空格（`\ ` → 空格）的还原，兼容会给含空格路径加引号的终端。
- **只有磁盘上确实存在的文件**才会被改写为绝对路径；非文件文本（如 `hello world`）原样返回，避免误伤。

### 接线到事件循环（`crates/tui/src/app.rs`）

- 新增 `use crate::composer;` 与 `paste_payload` 的 re-export。
- 在 `match ev` 中 `_ => {}` 之前新增 `Event::Paste(pasted)` 分支：`paste_payload(&pasted, &workdir)` 处理后用既有的 `composer::insert_str` 写入 `input`，渲染层 `render_composer` 自动回显。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 绝对路径已存在文件回显规范绝对路径 | `paste_existing_absolute_file_echoes_full_path` | `tui/src/app_helpers.rs` |
| 带末尾换行的绝对路径回显完整路径 | `paste_existing_file_with_trailing_newline_echoes_full_path` | `tui/src/app_helpers.rs` |
| 单/双引号包裹的绝对路径回显完整路径 | `paste_quoted_absolute_file_echoes_full_path` | `tui/src/app_helpers.rs` |
| 相对文件名按 workdir 解析为完整绝对路径 | `paste_existing_relative_file_resolves_against_workdir` | `tui/src/app_helpers.rs` |
| 不存在的绝对路径原样返回 | `paste_nonexistent_absolute_path_returned_verbatim` | `tui/src/app_helpers.rs` |
| 多行文本原样返回 | `paste_multiline_text_returned_verbatim` | `tui/src/app_helpers.rs` |
| 空内容原样返回 | `paste_empty_returned_verbatim` | `tui/src/app_helpers.rs` |
| 非文件普通文本原样返回 | `paste_non_file_text_returned_verbatim` | `tui/src/app_helpers.rs` |
| 退出序列含 disable-bracketed-paste | `write_restore_emits_all_restoration_sequences` | `tui/src/terminal.rs` |

## Gate

| 项 | 结果 |
|----|------|
| `cargo test --workspace` | 625 passed / 0 failed / 0 ignored |
| `cargo clippy --workspace --all-targets -- -D warnings` | 零警告 |
| `cargo build --workspace` | 零错误 |
