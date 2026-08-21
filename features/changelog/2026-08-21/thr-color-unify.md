# style(tui): workdir 与 tok/turn cost 标签改用 thr 状态栏标签色

## 变更

顶部标题的 `workdir` 段与底部边角 `[tok cost …] · [turn cost …]` 标签原先为
subtle 灰（低调层级），现统一改为状态栏 `thr` 前缀同款样式——
`theme::bold(theme::status_label_color())`（加粗亮蓝 LightBlue），与
`ctx (used/limit)` 计数同层级。`·` 分隔符保持 muted 近隐身，`跟随中…` 指示
器与模型名的 accent 不变。

- `crates/tui/src/theme.rs` `rounded_block_line_tok`：两段标签 subtle → 加粗亮蓝。
- `crates/tui/src/app_display.rs` `compose_top_title`：workdir 段 subtle → 加粗亮蓝。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| tok/turn cost 标签与 thr 同色同粗 | `tok_cost_corner_quiet_and_follow_indicator_accent`（断言更新为 fg+modifier 全匹配） | `crates/tui/src/render_tests/tok_cost.rs` |
| workdir 段加粗亮蓝（组合层） | `compose_title_segments_carry_graded_colors` | `crates/tui/src/app_display.rs` |
| workdir 段加粗亮蓝（display 集成层） | `compute_display_title_segments_carry_graded_colors` | `crates/tui/src/app_loop_tests/display_title_tests.rs` |

- 全量回归（本改动触及区）：`cargo test -p opencoder-tui` → lib 1504 passed / 0 failed，integration 1585 passed / 0 failed
- clippy（触及区）：`cargo clippy -p opencoder-tui --all-targets` → 零警告
- 说明：本轮期间另一并行会话正在移除 `theme` 配置项（`crates/core` 测试暂红，
  stash 本改动后复现同样错误，与本改动无关），故 workspace 级 gate 未在此轮闭合。
