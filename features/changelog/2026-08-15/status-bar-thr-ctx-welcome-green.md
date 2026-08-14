Commit: (working-tree, pre-initial-commit)

# 状态栏 thr/ctx 标签改为欢迎页同款绿色（加粗）

## 背景
用户对 `thr` / `ctx (x/x)` 标签的视觉色感经多轮调整（LightBlue → Indexed 色阶 → cargo Building 的加粗亮青）后仍不满意，最终定稿为与 TUI 欢迎页 `｛欢迎｝` 头部完全一致的语义色：`theme::ok_color()`（`Color::Green`）+ `Modifier::BOLD`。

期间发现一处接线缺陷：前序提交中 `LIGHT_BLUE` 常量虽被反复调值，但 `render_status.rs` 实际渲染一直走 `theme::accent()`，常量从未生效——本轮直接切到 `ok_color` 并彻底移除死掉的 `light_blue` 主题字段，消除「常量在、无人用」的漂移。

## 变更
### TUI 状态栏标签配色
- **`crates/tui/src/render_status.rs:70`**：`"thr "` 标签由 `theme::accent()` → `theme::ok_color()` + `Modifier::BOLD`（与 `welcome.rs` 头部同款）。
- **`crates/tui/src/render_status.rs:84`**：`ctx (used/limit)` 计数同改 `ok_color()` + BOLD；仪表条 + 百分比仍走 `theme::context_meter()` 阈值红黄绿语义色，颜色分工不变。
- **`crates/tui/src/theme.rs`**：删除未被任何渲染使用的 `LIGHT_BLUE` 常量、`Palette::light_blue` 字段、两个主题中的取值、`light_blue()` 访问器及对应断言（repair-on-touch：清除死代码而非再留一个无人消费的色槽）。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| thr/ctx 标签恒为欢迎绿、不随阈值压力变色；meter/percent 仍走阈值色 | status_bar_colors_split_between_meter_and_labels | crates/tui/src/render_tests/status_ctx.rs |
| ctx 分母为模型窗口而非压缩阈值 | status_bar_shows_ctx_percent | crates/tui/src/render_tests/status_ctx.rs |
| Dark 调色板与常量一致（light_blue 移除后仍成立） | palette_dark_matches_constants | crates/tui/src/theme.rs |

- 全量回归：`cargo test --workspace` → 156 套件全绿（2544 passed, 0 failed）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：theme.rs 581（迭代文件 ≤ 800）/ render_status.rs 114 / status_ctx.rs 107（新文件 ≤ 400）

## Impact Surface
- 用户可感知：TUI 状态栏 `thr` 与 `ctx (x/x)` 两处标签变为加粗绿色（与欢迎页标题同色），在任意上下文占用率下不再变色。
- 不影响：仪表条/百分比的阈值红黄绿语义、Light 主题其余配色、CLI/Web/session/store 边界。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
- [2026-08-14 status-bar 配色与 MCP 清理](../2026-08-14/status-bar-ctx-accent-and-mcp-cleanup.md)
