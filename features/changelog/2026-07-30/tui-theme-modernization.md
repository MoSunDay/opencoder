# refactor(tui): 集中化主题层（rounded 边框 + ctx 仪表 + 滚动条/细节现代化）

## 背景

TUI 各渲染模块（render / chat / welcome / help / command / menu / task / queue_panel / model_menu / cache_salt_menu / subagent_input）此前各自硬编码颜色与 `Block` 样式：边框样式、分隔符字符、状态栏 ctx 仪表、agent 标签前景色散落多处，无单一真相源。修改任一视觉常量需跨文件搜索，易遗漏且不一致。

## 变更

### 新增集中化主题层 `crates/tui/src/theme.rs`

纯函数式 styling 模块——零状态、零 class，全部为自由函数 + 常量：

- **语义调色板**（10 个 `pub const Color`）：`ACCENT`（Cyan）/ `WARN`（Yellow）/ `OK`（Green）/ `ERR`（Red）/ `INFO`（Blue）/ `MUTED`（DarkGray）/ `SUBTLE`（Gray）/ `TEXT`（White）/ `LOCAL`（Magenta）。16 色基线，广兼容。
- **Block 预设**（4 个 `pub fn`）：`rounded_block_plain()`（无标题圆角边框）/ `rounded_block(title)`（muted 边框）/ `rounded_block_focus(title)`（ACCENT 边框）/ `rounded_block_color(title, color)`（自定义色边框）。全部 `BorderType::Rounded`。
- **列表/状态辅助**（3 个 `pub fn`）：
  - `list_highlight()` — 选中行 256 色 `Indexed(238)` 背景 + BOLD，比裸 `DarkGray` 更柔和。
  - `context_meter(pct)` — 10 段 ctx 用量仪表：`▰`（U+25B0 填充）+ `▱`（U+25B1 空槽），阈值 `≥85 Red / ≥60 Yellow / else Green`（与原 `render_status` 内联逻辑完全一致，无语义漂移）。
  - `agent_chip_fg(agent)` — plan 模式 Yellow，否则 Cyan。
- **样式快捷**（4 个 `pub fn`）：`bold(color)` / `muted_style()` / `subtle_style()` / `local_style()`。

### 渲染层迁移（纯外观，无布局/交互变更）

以下模块改为引用 `crate::theme::*`，替换原有内联硬编码：

| 模块 | 迁移内容 |
|------|----------|
| `render.rs` | 面板 `rounded_block`；状态栏 ctx 仪表经 `context_meter`；agent chip 经 `agent_chip_fg` |
| `chat.rs` | 滚动条字形、分隔符、颜色常量 |
| `welcome.rs` | 边框、文本颜色 |
| `help.rs` | `rounded_block_focus`（ACCENT 边框聚焦态） |
| `command.rs` / `menu.rs` | `rounded_block` + `list_highlight` |
| `task.rs` | `rounded_block_plain` + `list_highlight` |
| `queue_panel.rs` | 颜色常量、分隔符 |
| `model_menu/view.rs` | `rounded_block_plain` |
| `cache_salt_menu/view.rs` | `rounded_block` |
| `subagent_input.rs` | 边框、颜色 |
| `local_cmd.rs` | `local_style`（local/non-context 信息着色） |

## 测试覆盖

| 功能 | 测试名 | 位置 |
|------|--------|------|
| ctx 仪表 0% = 10 空槽 + Green | `context_meter_zero_is_all_empty_and_green` | `crates/tui/src/theme.rs`（unit） |
| 59% 仍 Green（阈值下沿） | `context_meter_just_below_yellow_threshold_is_green` | 同上 |
| 60% Yellow（阈值上沿） + 6 段填充 | `context_meter_yellow_threshold_is_yellow` | 同上 |
| 84% 仍 Yellow | `context_meter_just_below_red_threshold_is_yellow` | 同上 |
| 85% Red（阈值上沿） | `context_meter_red_threshold_is_red` | 同上 |
| 100% = 10 填充 + Red | `context_meter_full_is_all_filled_and_red` | 同上 |
| 溢出 clamp 到 100 | `context_meter_clamps_overflow` | 同上 |
| agent chip plan=Yellow | `agent_chip_fg_plan_is_warn` | 同上 |
| agent chip 非 plan=Cyan | `agent_chip_fg_non_plan_is_accent` | 同上 |
| 选中行 Indexed(238) bg + BOLD | `list_highlight_has_indexed_bg_and_bold` | 同上 |
| bold 设置 fg + BOLD 修饰 | `bold_sets_fg_and_bold_modifier` | 同上 |
| muted/subtle/local 样式 | `muted_style_is_muted` 等 3 项 | 同上 |

间接覆盖（render_tests 委托）：状态栏 ctx 红色高用法 `status_bar_ctx_red_at_high_usage`（断言 `cell.fg == Color::Red`）。

### 全量回归

| 检查 | 结果 |
|------|------|
| `cargo test --workspace` | PASS — 1368 passed / 0 failed / 0 ignored（TUI lib 632 passed，含本变更 +14 新增 theme.rs 测试） |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS — 零警告 |
| `cargo build --workspace` | PASS — Finished |
| 防修绿扫描 | PASS — 无 `#[ignore]`、无删测试、无弱断言、无调试输出、无 TODO/FIXME、无硬编码密钥 |

## Impact Surface

- 纯外观变更：圆角边框、分隔符字符、滚动条字形、颜色集中化。无 `Constraint`/布局/`MouseHits` 命中矩形/交互逻辑改动。
- `context_meter` 的颜色阈值（85/60）与原 `render_status` 内联逻辑完全一致——无语义漂移。

## 行数

- `crates/tui/src/theme.rs`：202 行（< 400 新文件上限）
- `crates/tui/src/render.rs`：755 行 / `chat.rs`：778 行（< 800 迭代中上限）
