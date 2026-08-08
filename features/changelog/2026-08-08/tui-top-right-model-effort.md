Commit: (working-tree, pre-initial-commit)

# feat(tui): 顶部 body 标题 model/effort 右对齐至 ⬆ 箭头左侧

## 背景

顶部 body 标题原为左对齐整体字符串 `workdir · [mode] · model`（+ `·effort`）。当
workdir 路径较长时 model/effort 会被推到很靠右甚至溢出；用户要求把 model（+ 思考深度
effort）徽标移到标题行**最右端**、紧邻 jump-to-top `⬆` 箭头的左侧，`workdir · [mode]`
保持在左侧。

## 变更

- **`app_display.rs`**：新增纯函数 `compose_top_title(workdir, mode, model_bare,
  effort, row_width, arrow_w) -> Line<'static>` 与常量 `TOP_ARROW_W = 10`。
  - 左侧段 `workdir · [mode]` 贴左缘（首空格由 `theme::rounded_block_line` 补）。
  - 右侧段 `model`（+ ` ·effort`）右对齐：`pad = right_end − left_w − right_w − 1`，
    `right_end = row_width − arrow_w − 2`（保留 ⬆ 矩形占位 + 边界 `╮`）。
  - 窄行（`pad < 1`）丢弃右段，只返回 `workdir · [mode]`（绝不与 ⬆ 重叠）。
  - `arrow_w = 0`（scroll==0、⬆ 隐藏）时 model 紧贴右边界（col `row_width−2`）。
  - 颜色沿用原状态栏分段色：`[mode]` 用 `agent_chip_fg`，model/effort 用 `theme::text()`。
- **`app_loop.rs`**：`compute_display` 增参 `row_width: u16, arrow_w: u16`；顶层标题改由
  `super::app_display::compose_top_title(...)` 组合（原内联 span 拼接逻辑删除）。
- **`app.rs`**：调用点传入 `last_size.map_or(0, |(w,_)| w)`（row_width）与
  `scroll > 0` 时 `TOP_ARROW_W`（否则 0）。
- `render.rs` 的 ⬆ 标签 `"    ⬆    "` 不变（4 + wide⬆ + 4 = 10 列），由
  `TOP_ARROW_W` 守卫测试对齐。

## 测试清单（crates/tui，全部为 unit，纯函数 / TestBackend）

| 行为 | 测试名 | 层 |
| --- | --- | --- |
| 左段 `workdir · [mode]` + 右对齐裸 model id（去 provider 前缀） | `compute_display_title_strips_provider_prefix` | unit(app_loop) |
| model 后跟 `·effort` 徽标、右对齐 | `compute_display_title_with_effort_strips_prefix` | unit(app_loop) |
| 空白 reasoning_effort 省略徽标 | `compute_display_title_omits_blank_effort` | unit(app_loop) |
| 窄行丢弃右段、只留 `workdir · [mode]` | `compose_title_drops_right_segment_on_narrow_row` | unit(app_display) |
| 无 ⬆（arrow_w=0）时 model 紧贴右边界（右对齐换算） | `compose_title_hugs_right_border_without_arrow` | unit(app_display) |
| `TOP_ARROW_W` 与 render.rs `"    ⬆    "` 显示宽度一致 | `top_arrow_width_matches_label` | unit(app_display) |

## Gate

- 全量回归：`cargo test --workspace` → **2052 passed / 0 failed / 0 ignored**（当次实跑，
  隔离 target dir `.tgt-title-task`；基线 2024 + 本 scope 净新增 3（app_display 的
  `compose_title_drops_right_segment_on_narrow_row` / `compose_title_hugs_right_border_without_arrow`
  / `top_arrow_width_matches_label`）+ 既有 display_title_tests 3 例按新签名改写修复
  + 并行 agent 同树 in-flight 测试 +21 —— 净增计数含并行贡献，非本 scope 单独）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
- 行数：app_display.rs 251 / app_loop.rs 799 / app.rs 800 / render.rs 793（均 ≤ 800）
