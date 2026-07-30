# feat(tui): Ctrl+L 强制全屏重绘 + 输入 sanitize 剥离终端破坏性控制字符

## 背景

ratatui 通过 diff buffer 增量绘制：每帧只重绘变化的单元格。若用户粘贴的文本（或
程序化输入）含有 C0/C1/DEL 等终端控制字符，`Span::raw` 会原样输出，终端将其当作
控制码执行（移动光标、响铃、切换字符集…），导致实际显示与 diff buffer 记录不一致，
后续帧只重绘"变化"的单元格——已损坏的区域永远不会被修正，屏幕持续花屏。

此外，Ctrl+L 原先仅折叠 thinking/tool-output 块并清空输入框，未重置 diff buffer。
若屏幕已因上述控制字符或 alt-screen 切换而损坏，折叠操作无法修复——需要一次完整的
全屏重绘（清空 diff buffer，强制下一帧重绘每个单元格）。

## 变更

### 控制字符 sanitize — `crates/tui/src/composer.rs`

- **`is_corrupting_control(ch)`**：判定一个字符是否为终端破坏性控制字符——C0
  控制字符（`0x00`–`0x1F`，**排除** TAB `0x09` 与 LF `0x0A`）、DEL（`0x7F`）、
  C1 控制字符（`0x80`–`0x9F`）。CR（`0x0D`）被归类为 corrupting，使 `\r\n` 折叠
  为 `\n`。TAB 与 LF 是 composer 显式处理的合法文本，予以保留。
- **`pub fn sanitize(s) -> String`**：过滤 `is_corrupting_control` 为 true 的字符，
  保留 TAB、LF 及所有可打印文本（ASCII / CJK / emoji）。纯函数，无副作用。
- **`insert_str` 入口防护**：粘贴文本先经 `sanitize` 再插入，确保任何来源的粘贴
  都无法通过 `Span::raw` 破坏显示。
- **`insert_char` 入口防护**：单字符插入前检查 `is_corrupting_control`，若是则
  跳过（返回原文本 + 原游标），不插入。

### Ctrl+L 强制重绘 — `crates/tui/src/app_helpers.rs` + `crates/tui/src/app.rs`

- **`pre_key_intercept` 新增 `needs_clear: &mut bool` out-parameter**：Ctrl+L 分支
  在折叠块、清空输入后，将 `*needs_clear = true` 信号传回调用方。其他分支
  （Esc 退出 subagent 视图、Ctrl+U 不拦截）保持 `false`。
- **`apply_force_redraw` helper**（`app_helpers.rs`）：当 `needs_clear` 为 true 时
  执行 `terminal.clear()`（重置 diff buffer）+ `render_pending = true` +
  `skip_next_render = false`，使下一帧完整重绘每个单元格。从 `app.rs` 事件循环中
  提取为独立函数以保持文件行数合规。
- **`app.rs` 调用点**：`pre_key_intercept` 返回后调用 `apply_force_redraw`，替代
  原先内联的清屏逻辑。

### 注释清理 — `crates/tui/src/composer.rs`

- 修正 `is_corrupting_control` 内联注释：移除字面 TAB 字符 + 尾部空白，改为
  `// C0 except TAB and LF`；删除多余空行。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| sanitize 剥离 C0 控制字符（保留 TAB/LF） | `sanitize_strips_c0_controls` | `crates/tui/src/composer.rs`（unit） |
| sanitize 剥离 DEL + C1 控制字符 | `sanitize_strips_del_and_c1` | 同上 |
| sanitize 剥离 CR（`\r\n` → `\n`） | `sanitize_strips_carriage_return` | 同上 |
| sanitize 保留 TAB + LF | `sanitize_keeps_tab_and_newline` | 同上 |
| sanitize 保留正常文本（ASCII/CJK/emoji） | `sanitize_preserves_normal_text` | 同上 |
| insert_str 粘贴时 sanitize | `insert_str_sanitizes_pasted_text` | 同上 |
| insert_char 跳过控制字符 | `insert_char_skips_control_chars` | 同上 |
| Ctrl+L 触发 needs_clear + 清空输入；Ctrl+U 不拦截 | `ctrl_u_not_intercepted_ctrl_l_clears_input` | `crates/tui/src/app_helpers.rs`（unit） |
| apply_force_redraw：needs_clear=true 清空 diff buffer + 置位 render_pending / 清 skip_next_render | `apply_force_redraw_clears_terminal_and_sets_flags_when_needs_clear` | `crates/tui/src/app_helpers_tests/mod.rs`（unit） |
| apply_force_redraw：needs_clear=false 严格 no-op（flags 与终端均不变） | `apply_force_redraw_is_a_noop_when_needs_clear_false` | `crates/tui/src/app_helpers_tests/mod.rs`（unit） |

> 全部为 unit 层（源文件内联 `#[cfg(test)] mod tests`），零 I/O / DB / 网络依赖，
> 执行时间 < 10ms。用 `assert_eq!` 断言具体值，未复制实现逻辑自证。

## 全量回归

| 检查 | 结果 |
|------|------|
| `cargo test --workspace` | PASS — 1396 passed / 0 failed / 0 ignored |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS — 零警告 |
| `cargo build --workspace` | PASS — Finished |
| 防修绿扫描 | PASS — 无 `#[ignore]`、无删测试、无弱断言、无调试输出、无 TODO/FIXME、无硬编码密钥 |

## Impact Surface

- `composer.rs`：`sanitize` / `is_corrupting_control` / `insert_char` / `insert_str`
  行为变更——新增控制字符剥离。所有输入/粘贴入口受影响。
- `app_helpers.rs`：`pre_key_intercept` 签名变更（新增 `needs_clear` out-param）；
  新增 `apply_force_redraw` helper。
- `app.rs`：调用点更新（`apply_force_redraw` 替代内联清屏）。
- 无跨 crate 公开 API 变更；无数据库 / 网络交互。

## 行数

| 文件 | 行数 | 限制 | 合规 |
|------|------|------|------|
| `crates/tui/src/composer.rs` | 409 | ≤ 800（迭代中） | ✓ |
| `crates/tui/src/app.rs` | 758 | ≤ 800（迭代中） | ✓ |
| `crates/tui/src/app_helpers.rs` | 795 | ≤ 800（迭代中） | ✓ |
| `crates/tui/src/app_helpers_tests/mod.rs` | 498 | ≤ 800（迭代中） | ✓ |
