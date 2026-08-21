# style(tui): tok/turn cost 标签改用任务计时器同款 warn 色

## 变更

底部边角 `[tok cost …] · [turn cost …]` 两段标签由上一轮的加粗亮蓝
（`thr` 标签色）改为 `theme::warn_color()` 纯色 —— 与状态栏任务计时器
（`task_ms` 时长）和 running spinner/状态文字完全同色同层级（不加粗），
形成"运行开销"语义组。`·` 分隔符保持 muted，`跟随中…` 指示器与模型名
accent 不变；顶部 `workdir` 保持上一轮的加粗亮蓝不变。

- `crates/tui/src/theme.rs` `rounded_block_line_tok`：标签
  `bold(status_label_color())` → `Style::default().fg(warn_color())`。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| tok/turn cost 标签与任务计时器同色（fg+modifier 全匹配，非加粗） | `tok_cost_corner_quiet_and_follow_indicator_accent` | `crates/tui/src/render_tests/tok_cost.rs` |

- 回归（触及区）：`cargo test -p opencoder-tui --lib` → 1504 passed / 0 failed
- clippy（触及区）：`cargo clippy -p opencoder-tui --all-targets -- -D warnings` → 零警告
