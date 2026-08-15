Commit: (working-tree, pre-commit)

# ctx 仪表阈值调整：>80 Red / >40 Yellow（预警前移）

## 背景
状态栏 ctx 用量仪表（`theme::context_meter`）原阈值 ≥85 Red / ≥60 Yellow 偏保守：60% 以下长时间纯绿、80%+ 才见黄色，压缩临近时红色预警窗口过窄。调整为 **>40 即黄、>80 即红**，让中期占用更早可视化、红色预警窗口从 15% 扩到 19%。

## 变更
### `crates/tui/src/theme.rs` `context_meter(pct)`
- 颜色分支：`pct >= 85 → err` / `pct >= 60 → warn` 改为 **`pct > 80 → err` / `pct > 40 → warn`** / else ok。
- 段条构造（▰×filled + ▱×(10-filled)，`pct.min(100)`）与返回形状不变；唯一调用方 `render_status.rs:64` 仅消费 `(String, Color)`，无需改动。
- 测试：7 个 context_meter 测试数量不变，其中 4 个边界测试改名换边界值（40 绿上限 / 41 黄下限 / 80 黄上限 / 81 红下限），断言仍为具体段数 + 精确颜色。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| 0% 全空段 + 绿 | context_meter_zero_is_all_empty_and_green | crates/tui/src/theme.rs |
| 40% 绿上限（仍绿，4 段） | context_meter_at_green_ceiling_is_green | crates/tui/src/theme.rs |
| 41% 黄下限（转黄，4 段） | context_meter_just_above_green_threshold_is_warn | crates/tui/src/theme.rs |
| 80% 黄上限（仍黄，8 段） | context_meter_at_warn_ceiling_is_warn | crates/tui/src/theme.rs |
| 81% 红下限（转红，8 段） | context_meter_red_threshold_is_red | crates/tui/src/theme.rs |
| 100% 全满段 + 红 | context_meter_full_is_all_filled_and_red | crates/tui/src/theme.rs |
| 溢出钳制（255 按 100 处理） | context_meter_clamps_overflow | crates/tui/src/theme.rs |

- 定点：`cargo test -p opencoder-tui --lib context_meter` → 7 passed / 0 failed（过滤自 1309 lib 测试）
- 全量回归：`cargo test --workspace --no-fail-fast` → **163 套件全绿（2649 passed, 0 failed）**（含同工作区并行迭代代码）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → Finished
- 备注：首轮全量跑曾见 `opencoder-session --lib` 1 例失败（354 passed/1 failed），后续两次复跑均 355/355 全绿、未复现，位于并行迭代代码区域，与本改动（纯函数阈值）无关，标记 flaky 待观察。
- 行数：theme.rs 582（迭代文件 ≤ 800）

## Impact Surface
- 用户可感知：TUI 状态栏 ctx 仪表在 41%–80% 区间即显示黄色（原为绿色），>80% 变红（原 85%）；预警更早、更醒目。
- 不影响：仪表段数/字符、`render_status` 布局、`status_label_color()` 标签固定亮蓝、Light/Dark 主题其余配色、CLI/Web/session/store 边界。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
- [2026-08-15 状态栏 thr/ctx 标签绿色化](status-bar-thr-ctx-welcome-green.md)
