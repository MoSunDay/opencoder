Commit: 05d4bdf110cd7bfa75492f8ea7eebbb7cdb4c662

# feat(tui): 底部状态栏 mode 前状态点 + 压缩阈值与窗口用量双进度条

## 背景

底部状态栏原先按 `[mode] · <压缩阈值仪表> ctx <百分比/用量>  <耗时>  <spinner/status>`
渲染：mode 徽标没有前置指示点，进度条只有一根（基于 compaction_threshold 的压缩仪表），
ctx 文本只以数字报告模型窗口用量。用户要求在 **mode 之前加一个小点**，并在 **进度条与
ctx 之间再加一根进度条**，让压缩阈值与模型窗口用量各有一根可见仪表。

## 变更

- **`crates/tui/src/render.rs` `render_status`**：
  - mode 徽标前新增状态点 `●`（U+25CF，与 mode 徽标同色 `theme::agent_chip_fg`），
    布局变为 ` ● [mode] · …`。
  - 新增第二根 10 段进度条（**窗口用量仪表**）：基于 `win_pct`
    （`context_percent(used, context_limit, CONTEXT_BASELINE)`），用
    `theme::context_meter(win_pct)` 取色，插在原有压缩阈值仪表与 `ctx` 文本之间。
    顺序变为 ` ● [mode] · <压缩阈值仪表> <窗口用量仪表> ctx N% (used/limit) …`。
  - 原有压缩阈值仪表（bar_pct）与 ctx 文本颜色逻辑不变；注释由"进度条=压缩盘/数字=用量表"
    更新为"第一根=压缩盘、第二根=用量表、ctx 文本=数字"。
- **`crates/tui/src/render_tests/status_bar.rs`**：
  - `status_bar_omits_branding_and_top_moved_info` 前缀断言由 ` [act]` 更新为
    ` ● [act]`。
  - 新增 `status_bar_has_two_meters_before_ctx`（used=0 时两根仪表均为空、共 20 空段）。
  - 新增 `status_bar_budget_meter_tracks_window_not_threshold`（used 高于 80K 压缩阈值、
    低于 200K 窗口时：压缩仪表满 10 段、窗口仪表仅 ~9 段，证明两根仪表独立）。

## 测试清单（crates/tui，全部为 unit，TestBackend）

| 行为 | 测试名 | 层 |
| --- | --- | --- |
| 状态栏底左 ` ● [mode]` 前缀 + 无品牌/model + 保留 ctx | `status_bar_omits_branding_and_top_moved_info` | unit(render) |
| used=0 时两根仪表均为空、位于 mode 与 ctx 之间 | `status_bar_has_two_meters_before_ctx` | unit(render) |
| 窗口用量仪表独立于压缩阈值仪表（90% vs 100%） | `status_bar_budget_meter_tracks_window_not_threshold` | unit(render) |
| 既有 status_bar/status_ctx 全绿（spinner、耗时、ctx 红/绿、无 skill/steer/queue 徽章） | `status_bar_*` / `status_ctx_*` | unit(render) |

## Gate

- 全量回归：`cargo test --workspace` → **2093 passed / 0 failed**（EXIT=0）。
- 静态检查：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（EXIT=0）。
- 构建：`cargo build --workspace` → 成功（EXIT=0）。
- UI 定向：`render::tests::status_bar` 8 项全部通过；120 列 TestBackend 验证状态点、双仪表、ctx、耗时和 spinner 无重叠。
- 行数：`render.rs` 792 行，符合 ≤ 800 Gate。
