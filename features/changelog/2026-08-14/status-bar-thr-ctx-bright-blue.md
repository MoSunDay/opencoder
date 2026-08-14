Commit: (working-tree, pre-initial-commit)

# 状态栏 thr/ctx 标签由 cargo 亮绿改为同亮度淡亮蓝（ANSI 94）

## 背景
用户要求：`thr ctx (x/x)` 标签保留亮绿档位的亮度，但改成淡亮蓝。此前标签色为 cargo 状态色（ANSI 92 LightGreen，取自 cargo `Building` 行），与 meter 阈值色之外的信息混在一档绿色里；改用 ANSI 94（LightBlue）保持同一 9x 亮度层级 + BOLD，视觉亮度不变、色调换为淡亮蓝。

## 变更
### TUI 状态栏标签色
- **`crates/tui/src/theme.rs`**：`cargo_status_color()` → 重命名 `status_label_color()`，返回值 `Color::LightGreen`（92）→ `Color::LightBlue`（94）——名称不再绑 cargo 语义；仍主题无关（dark/light 下固定），doc 注释同步。单测 `cargo_status_color_is_ansi_bright_green` → `status_label_color_is_ansi_bright_blue`（含 light 主题下不变断言）。
- **`crates/tui/src/render_status.rs:73,87`**：`thr ` 前缀与 `ctx (used/limit)` 计数两处 span 改用 `theme::status_label_color()`；BOLD 修饰保留；注释同步。仪表条 + 百分比仍走 `theme::context_meter()` 阈值语义色（≥85 Red / ≥60 Yellow / else Green），色彩分离契约不变。
- **`crates/tui/src/render_tests/status_ctx.rs`**：色彩分离回归测试 `status_bar_colors_split_between_meter_and_labels` 的标签断言改指 `status_label_color()`，doc 注释与失败文案同步（"cargo-green" → "bright blue"）；meter/percent 的 `err_color` 断言不动。
- **`agents/tui/index.md`**：`theme` 条目「仅保留」硬编码色清单补上 `status_label_color()` 的 `LightBlue`（repair-on-touch）。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| 标签色 = ANSI 94 LightBlue，dark/light 主题下均固定 | `status_label_color_is_ansi_bright_blue` | crates/tui/src/theme.rs |
| meter/percent 走阈值色、thr/ctx 标签走亮蓝的色彩分离回归 | `status_bar_colors_split_between_meter_and_labels` | crates/tui/src/render_tests/status_ctx.rs |

- 全量回归：`cargo test --workspace` → 全绿（首轮 1 个偶发失败 `tools::bash::tests::bash_normal_completion`——真机进程时序型 flake，与本变更无关的 session crate；单测复跑 + 全量复跑均通过）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 干净
- 行数：theme.rs 582 ≤ 800 / render_status.rs 114 ≤ 400 / status_ctx.rs 106 ≤ 400（均为迭代中文件上限内）

## Impact Surface
- 用户可感知：TUI 状态栏 `thr` 前缀与 `ctx (x/x)` 计数从亮绿变为淡亮蓝（亮度层级与加粗不变）。
- 不影响：阈值 meter 条与百分比的语义色、mode chip、CLI / Web / session / store 各边界；`cargo_status_color` 无其它调用方（全仓唯一消费方即这两处 span）。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
- [同主题前序 changelog](../2026-08-14/status-bar-ctx-accent-and-mcp-cleanup.md)
