# feat(tui): Ctrl+C 空闲退出 + notepad 光标溢出钳制 + 文件树默认折叠

## 背景

1. Ctrl+C 在 idle 时为 no-op（`KeyAction::None`），与 Ctrl+D 退出语义不一致；用户空闲态按 Ctrl+C
   预期退出，运行中才应中断 turn。
2. notepad 编辑器 / 终端 `place_cursor` 用裸 `+` 计算 `(x, y)`，超长行或超多行时 u16 溢出可致光标
   逃逸面板或 panic。
3. notepad 文件树默认展开整棵目录，深层 / 巨型目录（如 node_modules）扫描慢且无上限。

## 变更

### Ctrl+C idle 退出
- **`key_handler.rs`**：`cancel` 键 `running → Cancel`，`idle → Quit`（原 `None`）。注释说明语义。
  两处映射（正常 composer + subagent-focus 禁用输入路径）同步。
- **`keymap.rs`**：`KEYMAP_INFO` cancel 描述补 ` / Quit when idle`。

### notepad 光标溢出钳制
- **`notepad/editor.rs`** `set_editor_cursor`：`row/col` 先 `.min(u16::MAX)` 再 `saturating_add`，
  最后 `x.min(inner.right()-1)` / `y.min(inner.bottom()-1)` 钳进面板。
- **`notepad/mod.rs`** `place_cursor`（Terminal focus）：`str_width` 先 `.min(u16::MAX)`，
  `saturating_add` 链 + `x.min(max_x)` 钳制。

### notepad 文件树默认折叠 + 安全上限
- **`notepad/tree.rs`**：
  - 目录默认 collapsed（lazy 展开）；`rebuild` 改为收集 **expanded** 集合（原收集 collapsed）。
  - 新增 `MAX_DEPTH=32` / `MAX_TOTAL_ENTRIES=5000` 两道安全阀，`build_recursive` 超深即停。
  - `total = lines.len()` 去掉 `.max(1)`（dead clamp）。
  - 测试更新：默认折叠断言、`expand_shows_children`、`collapse_hides_children` 两步操作、
    `rebuild_preserves_expansion`（原 `rebuild_preserves_collapse`）。

### 测试重组
- **`app_tests/key_tests.rs`**：移除旧 `ctrl_c_does_not_quit` / `ctrl_d_quits` 等（语义已变）。
- **`app_tests/key_tests_quit.rs`**（新）：Ctrl+C 三种编码（chord / raw ETX / Kitty）×
  (idle 退出 / running 中断) × (composer / subagent-focus) 全覆盖 + Ctrl+D。
- **`notepad/render_tests.rs`**（新）：超长行 / 超多行光标不溢出、command 模式无光标、3×2 极小终端渲染。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| idle Ctrl+C 退出 | `ctrl_c_quits_when_idle` | app_tests/key_tests_quit.rs |
| running Ctrl+C 中断 | `ctrl_c_cancels_when_running` | app_tests/key_tests_quit.rs |
| subagent-focus Ctrl+C 退出/中断 | `subagent_ctrl_c_*` | app_tests/key_tests_quit.rs |
| 超长行光标不溢出 | `render_editor_long_line_cursor_no_overflow` | notepad/render_tests.rs |
| 超多行光标不溢出 | `render_editor_many_lines_cursor_no_overflow` | notepad/render_tests.rs |
| 文件树默认折叠 | `*` | notepad/tree.rs |
| 展开显示子项 | `expand_shows_children` | notepad/tree.rs |
| rebuild 保留展开态 | `rebuild_preserves_expansion` | notepad/tree.rs |

- 全量回归：`cargo test --workspace` → 全绿
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：notepad/tree.rs 446 ≤ 800；render_tests.rs 213 ≤ 400；key_tests_quit.rs 131 ≤ 400

## Impact Surface
- Ctrl+C 行为变更（idle 退出）：用户可感知；运行中语义不变。
- notepad 光标 / 文件树：仅 notepad 视图内部，不影响 CLI/Web/session/store 边界。
