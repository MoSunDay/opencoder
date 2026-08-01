Commit: (working-tree, pre-initial-commit)

# fix(tui): 模式切换改 Ctrl+T，强制刷屏独立到 Ctrl+F

## 背景

- 模式切换（act <-> plan 纯切换、保留上下文）此前绑定 Ctrl+U，与 Ctrl+L 的
  "清屏" 语义在记忆上相互干扰，且 Ctrl+U 在部分 shell/终端中另有 readline
  kill-line 惯例。
- Ctrl+L 此前一键承担"强制全屏重绘 + 退出子代理视图 + 折叠所有输出 + 清空输入"
  四重职责。强制重绘（清空 ratatui diff buffer）是终端花屏/控制字符损坏场景下
  的修复手段，与"折叠/清空"的日常操作频率不同，混绑在单一按键上难以按需使用。

## 变更

### 模式切换：Ctrl+U → Ctrl+T（`crates/tui/src/key_handler.rs`）

- `handle_key` 的 CONTROL 分支中，`KeyCode::Char('u')` 改为 `KeyCode::Char('t')`，
  行为不变：act <-> plan 纯切换（`SwitchAgentNoClear`），保留完整 transcript，
  不触碰输入框。subagent-focus（`input_disabled`）下同样为 no-op。
- Ctrl+U 不再绑定任何动作（落入 CONTROL 分支默认 no-op）。

### 强制刷屏：Ctrl+L → Ctrl+F（`crates/tui/src/app_helpers.rs`）

- `pre_key_intercept` 中 Ctrl+L 处理块移除 `*needs_clear = true` 与对应注释：
  Ctrl+L 保留 退出子代理视图 / 折叠 thinking 与 tool-output 块 / 清空输入框。
- 新增 Ctrl+F 分支：消费按键并置 `*needs_clear = true`，由既有
  `apply_force_redraw` 清空终端 diff buffer、置 `render_pending`、清
  `skip_next_render`，强制下一帧全屏重绘；不触碰输入框与光标。

### 提示文案（`crates/tui/src/keybind.rs`、`crates/tui/src/welcome.rs`）

- `Ctrl+U 仅切换状态` → `Ctrl+T 仅切换状态，保留上下文（当 Ctrl+Shift+Tab 被拦截时使用）`。
- 帮助列表将 Ctrl+L 拆为两行：`Ctrl+F 强制重新渲染屏幕`、
  `Ctrl+L 退出子代理视图 / 折叠所有输出 / 清空输入`。
- welcome 教程同步 Ctrl+T。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| Ctrl+T act→plan 纯切换不清 transcript/输入框 | `ctrl_t_switches_mode_without_clear` | `crates/tui/src/app_tests/key_tests.rs`（unit） |
| Ctrl+T plan→act 纯切换 | 同上（plan 方向） | 同上 |
| subagent-focus 下 Ctrl+T 为 no-op | `ctrl_t_blocked_when_input_disabled` | 同上 |
| Ctrl+T 不被 `pre_key_intercept` 消费（落到 handle_key 切模式） | `ctrl_t_not_intercepted_ctrl_l_clears_ctrl_f_redraws` | `crates/tui/src/app_helpers_tests/mod.rs`（unit） |
| Ctrl+L 折叠/退出子代理/清空输入，但不再触发强制重绘 | 同上（Ctrl+L 段） | 同上 |
| Ctrl+F 仅触发强制重绘，不动输入框/光标 | 同上（Ctrl+F 段） | 同上 |
| apply_force_redraw 管线在 needs_clear=true 下清 diff buffer + 置位 | `apply_force_redraw_clears_terminal_and_sets_flags_when_needs_clear` | `crates/tui/src/app_helpers_tests/mod.rs`（unit） |
| apply_force_redraw 在 needs_clear=false 下严格 no-op | `apply_force_redraw_is_a_noop_when_needs_clear_false` | 同上 |

> 全部 unit 层，零 I/O / DB / 网络依赖。

## 全量回归

| 检查 | 结果 |
|------|------|
| `cargo check --workspace`（非 test） | PASS — Finished |
| `cargo test -p opencoder-tui --lib -- ctrl_t apply_force_redraw` | PASS — 5 passed / 0 failed |
| `cargo test -p opencoder-tui --lib` 全量 | PASS — 722 passed / 0 failed |
| `cargo test --workspace` 全量 | PASS — 1500 passed / 0 failed |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS — 零警告 |
| `cargo build --workspace` | PASS — Finished |

> 以上为并行重构（store `task_type` / subagent block 模块化 / mouse 测试）合入后的
> 最终收敛实跑快照；本变更文件（key_handler / app_helpers / keybind / welcome /
> key_tests / app_helpers_tests）与此前独立验证结果一致。
